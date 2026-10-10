//! A `MoQ` Transport draft-18 client over WebTransport, built on `xmoq-moqt`.
//!
//! It subscribes to one track and prints the Objects it receives, or publishes
//! one Object. What it prints to stdout is a contract with the scripts that
//! drive it; see `report`.

mod hex;
mod io;
mod report;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::Instant;
use wtransport::Connection;
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, ReasonPhrase, RequestError, publish_done_status, setup_option,
};
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter, SubscriptionFilter};
use xmoq_moqt::session::{
    RequestEvent, RequestId, RequestState, Role, Session, SubscribedAlias, Transport,
};

use crate::io::{DataEvent, ObjectInbox, RequestStream, RequestStreamEvent, Trace};

/// Track Alias this client announces in PUBLISH. It publishes one track per
/// session, so any value is unique (§11.1).
const PUBLISH_TRACK_ALIAS: u64 = 1;

/// How long the peer gets to acknowledge the close before the process exits.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// How long the last Objects of a subscription are waited for after
/// `PUBLISH_DONE`, when fewer data streams ended than it counted (§10.11).
///
/// Provisional: the draft asks for a timer and names no duration, and `XmoQ` has
/// not decided one.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

enum Mode {
    /// Prints this many Objects, then leaves.
    Subscribe { objects: u64 },
    /// Publishes one Object carrying this payload.
    Publish { payload: Vec<u8> },
}

struct Args {
    mode: Mode,
    url: String,
    cert_sha256: [u8; 32],
    namespace: String,
    track_name: String,
    timeout_secs: u64,
    trace: Trace,
}

impl Args {
    /// Splits `a/b` into the namespace fields `a` and `b`. The slash is a
    /// convention of this command line, shared with moqtail's tools; the draft
    /// gives a Track Namespace no text form with slashes.
    fn track_namespace(&self) -> Result<TrackNamespace> {
        let fields = self
            .namespace
            .split('/')
            .filter(|field| !field.is_empty())
            .map(|field| field.as_bytes().to_vec())
            .collect();
        Ok(TrackNamespace::new(fields)?)
    }

    fn full_track_name(&self) -> Result<FullTrackName> {
        Ok(FullTrackName::new(
            self.track_namespace()?,
            self.track_name.as_bytes().to_vec(),
        )?)
    }
}

fn parse_args() -> Result<Args> {
    let mut mode = None;
    let mut payload_hex = None;
    let mut objects = 1;
    let mut url = None;
    let mut cert_sha256 = None;
    let mut namespace = None;
    let mut track_name = None;
    let mut timeout_secs = 20;
    let mut trace = false;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--trace" {
            trace = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| anyhow!("missing value for {flag}"))?;
        match flag.as_str() {
            "--mode" => mode = Some(value),
            "--payload-hex" => payload_hex = Some(value),
            "--objects" => objects = value.parse().context("--objects")?,
            "--url" => url = Some(value),
            "--cert-sha256" => cert_sha256 = Some(value),
            "--namespace" => namespace = Some(value),
            "--track" => track_name = Some(value),
            "--timeout-secs" => timeout_secs = value.parse().context("--timeout-secs")?,
            other => bail!("unknown flag {other}"),
        }
    }

    let mode = match mode.context("--mode is required")?.as_str() {
        "subscribe" => Mode::Subscribe { objects },
        "publish" => Mode::Publish {
            payload: hex::decode(&payload_hex.context("--payload-hex is required to publish")?)?,
        },
        other => bail!("unknown --mode {other}"),
    };
    let cert_sha256 = hex::decode(&cert_sha256.context("--cert-sha256 is required")?)?
        .try_into()
        .map_err(|_| anyhow!("--cert-sha256 must be 32 bytes"))?;

    Ok(Args {
        mode,
        url: url.context("--url is required")?,
        cert_sha256,
        namespace: namespace.context("--namespace is required")?,
        track_name: track_name.context("--track is required")?,
        timeout_secs,
        trace: Trace::new(trace),
    })
}

/// What a response other than the expected one means for the caller.
fn refused(what: &str, event: RequestEvent) -> anyhow::Error {
    match event {
        RequestEvent::Rejected(RequestError {
            error_code,
            retry_interval,
            error_reason,
            ..
        }) => anyhow!(
            "{what} rejected: error_code={error_code:#x} retry_interval={retry_interval} reason={:?}",
            String::from_utf8_lossy(error_reason.as_bytes()),
        ),
        other => anyhow!("unexpected response to {what}: {other:?}"),
    }
}

/// The next event of a request stream, or never when the stream is gone.
async fn next_request_event(stream: &mut Option<RequestStream>) -> Result<RequestStreamEvent> {
    match stream {
        Some(stream) => stream.next_event().await,
        None => std::future::pending().await,
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// One session and what the two modes need to use it.
struct Client<'a> {
    connection: &'a Connection,
    session: Session,
    objects: ObjectInbox,
    args: &'a Args,
}

impl Client<'_> {
    /// Sends a request on a new stream and returns the event its response
    /// produces. On any failure the request is cancelled, in the session and on
    /// the stream, so it does not stay open on either.
    async fn request(
        &mut self,
        request_id: RequestId,
        message: &ControlMessage,
        label: &str,
    ) -> Result<(RequestStream, RequestEvent)> {
        let mut stream = match RequestStream::open(self.connection, self.args.trace, label).await {
            Ok(stream) => stream,
            Err(error) => {
                self.session.cancel(request_id);
                return Err(error);
            }
        };
        let response = async {
            stream.send(message).await?;
            match stream.next_event().await? {
                RequestStreamEvent::Message(response) => {
                    Ok(self.session.on_request_message(request_id, response)?)
                }
                RequestStreamEvent::Closed(how) => {
                    bail!("the peer closed the {label} stream without a response: {how}")
                }
            }
        };
        match response.await {
            Ok(event) => Ok((stream, event)),
            Err(error) => {
                self.session.cancel(request_id);
                stream.cancel();
                Err(error)
            }
        }
    }

    async fn subscribe(&mut self, wanted: u64) -> Result<()> {
        let (request_id, message) = self.session.subscribe(
            self.args.full_track_name()?,
            vec![
                Parameter::Forward(true),
                Parameter::SubscriberPriority(128),
                Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
                Parameter::GroupOrder(GroupOrder::Ascending),
            ],
        )?;
        let (stream, event) = self.request(request_id, &message, "subscribe").await?;
        let track_alias = match event {
            RequestEvent::SubscribeEstablished { track_alias, .. } => track_alias,
            RequestEvent::UnsupportedTrack {
                property_type,
                track_alias,
            } => {
                // §2.5.1: the session already ended the request; the stream is
                // cancelled to tell the publisher, and so are the subgroup
                // streams it may have opened already.
                self.objects.stop_track(track_alias);
                stream.cancel();
                bail!("the track needs property {property_type:#x}, which this client lacks");
            }
            other => {
                stream.cancel();
                return Err(refused("SUBSCRIBE", other));
            }
        };
        report::subscribed(track_alias);
        self.receive_objects(request_id, stream, track_alias, wanted)
            .await
    }

    /// Prints Objects of the subscription until `wanted` arrived or the
    /// subscription ended. The request stream is read all the while: it carries
    /// `PUBLISH_DONE`, and the peer cancels the subscription by closing it
    /// (§11.4.1).
    async fn receive_objects(
        &mut self,
        request_id: RequestId,
        stream: RequestStream,
        track_alias: u64,
        wanted: u64,
    ) -> Result<()> {
        let mut stream = Some(stream);
        let mut drain_deadline = None;
        let mut drain_timed_out = false;
        let mut received = 0;

        while received < wanted {
            tokio::select! {
                event = self.objects.next_event() => match event? {
                    DataEvent::Object(header, object) => {
                        if self.belongs_to(header.track_alias, track_alias) {
                            report::object(&header, &object);
                            received += 1;
                        } else {
                            report::ignored(header.track_alias);
                        }
                    }
                    DataEvent::StreamClosed { track_alias } => {
                        self.session.on_data_stream_closed(track_alias);
                    }
                    DataEvent::TrackMalformed { track_alias: malformed, error } => {
                        report::track_malformed(malformed, &error);
                        if malformed == track_alias {
                            // §2.4.2: the subscription is cancelled. The session
                            // is not at fault and is closed without an error.
                            self.session.cancel(request_id);
                            self.objects.stop_track(track_alias);
                            if let Some(stream) = stream.take() {
                                stream.cancel();
                            }
                            return Err(error.into());
                        }
                    }
                },
                event = next_request_event(&mut stream) => match event? {
                    RequestStreamEvent::Message(message) => {
                        match self.session.on_request_message(request_id, message)? {
                            RequestEvent::SubscriptionDone(done) => {
                                report::publish_done_received(&done);
                                drain_deadline = Some(Instant::now() + DRAIN_TIMEOUT);
                            }
                            other => bail!("unexpected message on the subscribe stream: {other:?}"),
                        }
                    }
                    RequestStreamEvent::Closed(how) => {
                        let closed = stream.take();
                        if self.session.request_state(request_id) != Some(RequestState::Terminated) {
                            self.session.cancel(request_id);
                            self.objects.stop_track(track_alias);
                            if let Some(closed) = closed {
                                closed.cancel();
                            }
                            report::subscription_ended("request stream closed", received, wanted);
                            bail!("the peer closed the subscribe stream without PUBLISH_DONE: {how}");
                        }
                        // After PUBLISH_DONE the publisher closes the stream
                        // (§10.11). Dropping this end answers with a FIN.
                    }
                },
                () = sleep_until(drain_deadline) => {
                    self.session.on_drain_timeout(track_alias);
                    drain_deadline = None;
                    drain_timed_out = true;
                }
            }

            let done = self.session.request_state(request_id) == Some(RequestState::Terminated);
            if done && self.session.subscribed_alias(track_alias) == SubscribedAlias::Retired {
                let why = if drain_timed_out {
                    "PUBLISH_DONE, drain timeout"
                } else {
                    "PUBLISH_DONE, all streams closed"
                };
                self.objects.stop_track(track_alias);
                report::subscription_ended(why, received, wanted);
                return Ok(());
            }
        }

        // Enough Objects: the subscription is no longer wanted. Nothing happens
        // in the session when PUBLISH_DONE already ended the request.
        self.session.cancel(request_id);
        self.objects.stop_track(track_alias);
        if let Some(stream) = stream {
            stream.cancel();
        }
        Ok(())
    }

    fn belongs_to(&self, track_alias: u64, subscribed: u64) -> bool {
        match self.session.subscribed_alias(track_alias) {
            // §10.11: Objects of a subscription that PUBLISH_DONE ended are still
            // its Objects while the alias drains.
            SubscribedAlias::Established(_) | SubscribedAlias::Draining(_) => {
                track_alias == subscribed
            }
            // §11.1: late Objects of an ended subscription are dropped quietly;
            // §11.4.2: so may be a stream with an alias nothing established.
            SubscribedAlias::Retired | SubscribedAlias::Unknown => false,
        }
    }

    /// Announces the namespace and offers the track. The returned streams have
    /// to stay open for as long as the track is published.
    async fn offer_track(&mut self) -> Result<(RequestStream, RequestId, RequestStream)> {
        let (namespace_request, message) = self
            .session
            .publish_namespace(self.args.track_namespace()?, vec![])?;
        let (namespace_stream, event) = self
            .request(namespace_request, &message, "publish-namespace")
            .await?;
        match event {
            RequestEvent::PublishNamespaceEstablished => report::namespace_published(),
            other => {
                namespace_stream.cancel();
                return Err(refused("PUBLISH_NAMESPACE", other));
            }
        }

        let (publish_request, message) = self.session.publish(
            self.args.full_track_name()?,
            PUBLISH_TRACK_ALIAS,
            vec![Parameter::Forward(true)],
            vec![],
        )?;
        let (publish_stream, event) = self.request(publish_request, &message, "publish").await?;
        match event {
            RequestEvent::PublishEstablished { forward, .. } => {
                report::published(PUBLISH_TRACK_ALIAS, forward);
            }
            other => {
                publish_stream.cancel();
                return Err(refused("PUBLISH", other));
            }
        }
        Ok((namespace_stream, publish_request, publish_stream))
    }

    /// Sends one Object as Group 0, Object 0 on a new subgroup stream.
    async fn send_object(&self, payload: &[u8]) -> Result<()> {
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias: PUBLISH_TRACK_ALIAS,
            group_id: 0,
            subgroup_id: SubgroupId::Zero,
            publisher_priority: Some(0),
            properties: false,
            // This Object is the only one of its Group, so the stream holds the
            // Group's largest Object and its first.
            end_of_group: true,
            first_object: true,
        });
        encoder.object(&SubgroupObject {
            object_id: 0,
            properties: vec![],
            status: ObjectStatus::Normal,
            payload: payload.to_vec(),
        })?;
        let mut data = self.connection.open_uni().await?.await?;
        data.write_all(&encoder.take_bytes()).await?;
        // §11.4.3: every Object of the Subgroup was written, so the stream ends
        // with a FIN.
        data.finish().await?;
        report::sent(payload.len());
        Ok(())
    }

    async fn publish(&mut self, payload: &[u8]) -> Result<()> {
        let (_namespace_stream, publish_request, mut publish_stream) = self.offer_track().await?;

        // The driver writes one line once the subscriber is ready, and closes
        // stdin once it has read the Object; the session has to stay open in
        // between. The request stream is read all the while: the subscriber
        // may update the subscription on it, or cancel by closing it.
        let mut stdin = BufReader::new(tokio::io::stdin()).lines();
        let mut stream_count = 0;
        let status_code = loop {
            tokio::select! {
                line = stdin.next_line() => match line? {
                    Some(_) if stream_count == 0 => {
                        self.send_object(payload).await?;
                        stream_count = 1;
                    }
                    Some(_) => {}
                    None if stream_count == 0 => bail!("stdin closed before the send trigger"),
                    None => break publish_done_status::TRACK_ENDED,
                },
                event = publish_stream.next_event() => match event? {
                    RequestStreamEvent::Message(message) => {
                        match self.session.on_request_message(publish_request, message)? {
                            // §10.9.1: a refused update also ends the
                            // subscription, with UPDATE_FAILED.
                            RequestEvent::UpdateRefused { response } => {
                                publish_stream.send(&response).await?;
                                report::update_refused();
                                break publish_done_status::UPDATE_FAILED;
                            }
                            other => bail!("unexpected message on the publish stream: {other:?}"),
                        }
                    }
                    RequestStreamEvent::Closed(how) => {
                        self.session.cancel(publish_request);
                        publish_stream.cancel();
                        bail!("the subscriber ended the subscription: {how}");
                    }
                },
            }
        };

        // §10.11: PUBLISH_DONE goes out after the data streams are closed, and
        // the request stream is closed right after it. `send_object` closes the
        // one data stream before it returns.
        let done = self.session.publish_done(
            publish_request,
            status_code,
            stream_count,
            ReasonPhrase::default(),
        )?;
        publish_stream.send(&done).await?;
        publish_stream.finish().await
    }

    async fn run_mode(&mut self) -> Result<()> {
        match &self.args.mode {
            Mode::Subscribe { objects } => self.subscribe(*objects).await,
            Mode::Publish { payload } => self.publish(payload).await,
        }
    }
}

async fn run_session(connection: &Arc<Connection>, args: &Args) -> Result<()> {
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let (mut session_inbox, objects, _readers) = io::start_readers(connection.clone(), args.trace);

    // §3.3: each peer opens one control stream that begins with SETUP and stays
    // open for the whole session, so `control` lives until this function returns.
    let mut control = connection.open_uni().await?.await?;
    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    let setup = session.setup(vec![KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        implementation.as_bytes().to_vec(),
    )?])?;
    io::send_message(&mut control, &setup, args.trace, "control").await?;

    let peer_setup = session.on_control_message(session_inbox.next_control_message().await?)?;
    report::setup_ok(&peer_setup);

    let mut client = Client {
        connection,
        session,
        objects,
        args,
    };
    // The session inbox is watched for the rest of the session: it reports a
    // closed control stream, a failed stream reader and any further control
    // message, each of which ends the session.
    let unexpected = tokio::select! {
        result = client.run_mode() => return result,
        event = session_inbox.next_control_message() => event?,
    };
    // After SETUP no message this crate decodes belongs on the control stream,
    // so this returns the protocol error to close with.
    client.session.on_control_message(unexpected)?;
    bail!("the session accepted a second control message")
}

/// The code to close the session with (§3.5): the one the protocol error asks
/// for, or `NO_ERROR` for a normal end and for failures that are not the peer's.
fn termination_code(result: &Result<()>) -> u32 {
    let Err(error) = result else {
        return 0;
    };
    // WHY walk the chain: `anyhow::Error::context` wraps the cause, and a
    // protocol error has to be found under any number of such wrappers.
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<xmoq_moqt::Error>())
        .and_then(xmoq_moqt::Error::termination_code)
        .map_or(0, xmoq_moqt::TerminationCode::code)
}

async fn run(args: &Args) -> Result<()> {
    // WHY one deadline for connecting and the session: `--timeout-secs` bounds
    // the whole run, and a relay that never answers the handshake must not
    // outlast it.
    let deadline = Instant::now() + Duration::from_secs(args.timeout_secs);
    let timed_out = || anyhow!("timed out after {}s", args.timeout_secs);

    let connection = tokio::time::timeout_at(deadline, io::connect(&args.url, args.cert_sha256))
        .await
        .map_err(|_| timed_out())??;
    report::connected(&args.url);

    // WHY the timeout sits here and not around `run`: the close below has to
    // happen on every path, a timeout included.
    let result = match tokio::time::timeout_at(deadline, run_session(&connection, args)).await {
        Ok(result) => result,
        Err(_) => Err(timed_out()),
    };
    connection.close(termination_code(&result).into(), b"");
    let _ = tokio::time::timeout(CLOSE_GRACE, connection.closed()).await;
    result
}

#[tokio::main]
async fn main() -> Result<()> {
    run(&parse_args()?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_with_the_code_of_the_protocol_error() {
        let violation = || xmoq_moqt::Error::ProtocolViolation("test");
        let cases: Vec<(Result<()>, u32)> = vec![
            (Ok(()), 0x0),
            (Err(anyhow!("stdin closed")), 0x0),
            (Err(violation().into()), 0x3),
            (
                Err(anyhow::Error::from(violation()).context("reading")),
                0x3,
            ),
            (Err(xmoq_moqt::Error::DuplicateTrackAlias(7).into()), 0x5),
            (
                Err(xmoq_moqt::Error::UnsupportedMessageType(0x10).into()),
                0x1,
            ),
            (
                Err(xmoq_moqt::Error::BufferLimitExceeded { limit: 8 }.into()),
                0x1,
            ),
        ];
        for (result, code) in cases {
            assert_eq!(termination_code(&result), code, "{result:?}");
        }
    }
}
