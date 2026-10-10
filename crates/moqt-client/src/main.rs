//! A `MoQ` Transport draft-18 client over WebTransport, built on `xmoq-moqt`.
//!
//! It subscribes to one track and prints the Objects it receives, or publishes
//! one Object. What it prints to stdout is a contract with the scripts that
//! drive it; see `report`.

mod hex;
mod io;
mod report;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::Instant;
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, ReasonPhrase, RequestError, publish_done_status, request_error_code,
    setup_option,
};
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter, SubscriptionFilter};
use xmoq_moqt::session::{
    PeerRequest, PeerRequestEvent, PeerRequestId, PeerRequestOpened, RequestEvent, RequestId,
    RequestState, Role, Session, SubscribedAlias, Transport,
};

use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::{self, WtConnection};
use xmoq_moqt_io::{
    Connection, Direction, PeerRequestStreamEvent, RequestClose, RequestStreamEvent, SendStream,
    StreamEnd, StreamStops, Trace, UniEvent,
};

use crate::io::{DataInbox, RequestStream};

/// Track Alias this client announces in PUBLISH. It publishes one track per
/// session, so any value is unique (§11.1).
const PUBLISH_TRACK_ALIAS: u64 = 1;

/// How long the peer gets to acknowledge the close before the process exits.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// How long the peer gets to take what was written to a stream before this
/// client goes on, which in the end means closing the session. Provisional.
///
/// WHY wait at all: closing the session drops what the transport has not
/// delivered yet, and this client closes right after its last write.
const DELIVERY_WAIT: Duration = Duration::from_secs(5);

/// How long the subscriber gets to end its direction of the request stream
/// after this client's `PUBLISH_DONE`, before the session is closed without
/// knowing whether everything arrived. Provisional.
///
/// WHY short: a peer that follows only the draft may never end it, and every
/// run against such a peer waits this long.
/// WHY longer than the 2 seconds an `XmoQ` relay waits for the counted
/// streams: the relay's reset at the end of its wait is to be seen, not cut
/// off by this client's own limit.
const PUBLISH_DONE_ANSWER_WAIT: Duration = Duration::from_secs(3);

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
    /// Announces the namespace with `PUBLISH_NAMESPACE` and sends no PUBLISH:
    /// the track is published to whoever sends SUBSCRIBE for it, which is how
    /// a relay asks for a track of an announced namespace (§9.5).
    PublishNamespace { payload: Vec<u8> },
}

struct Args {
    mode: Mode,
    url: String,
    cert_sha256: [u8; 32],
    namespace: String,
    track_name: String,
    timeout_secs: u64,
    /// How long the publisher waits between the answer to its
    /// `PUBLISH_NAMESPACE` and its PUBLISH.
    publish_delay: Duration,
    /// How long a request of the peer waits for its refusal.
    ///
    /// WHY the two delays: on a real link, a SUBSCRIBE the relay sends after
    /// it accepted the namespace is answered a round trip later, and the
    /// PUBLISH is already under way by then (§5.1 has the SUBSCRIBE give way).
    /// On loopback that window is too short to hit on purpose; the delays
    /// widen it.
    answer_delay: Duration,
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
    let mut publish_delay_ms = 0;
    let mut answer_delay_ms = 0;
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
            "--publish-delay-ms" => {
                publish_delay_ms = value.parse().context("--publish-delay-ms")?;
            }
            "--answer-delay-ms" => {
                answer_delay_ms = value.parse().context("--answer-delay-ms")?;
            }
            other => bail!("unknown flag {other}"),
        }
    }

    let mode = match mode.context("--mode is required")?.as_str() {
        "subscribe" => Mode::Subscribe { objects },
        "publish" => Mode::Publish {
            payload: hex::decode(&payload_hex.context("--payload-hex is required to publish")?)?,
        },
        "publish-namespace" => Mode::PublishNamespace {
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
        publish_delay: Duration::from_millis(publish_delay_ms),
        answer_delay: Duration::from_millis(answer_delay_ms),
        trace: if trace {
            Trace::new(report::trace)
        } else {
            Trace::default()
        },
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
    connection: &'a WtConnection,
    session: Session,
    objects: DataInbox,
    /// Stops the readers of subgroup streams this client no longer wants.
    stops: StreamStops,
    /// The Track Alias of every subgroup stream that is being read.
    open_streams: HashMap<u64, u64>,
    /// The requests the peer opens, in the mode that serves them.
    peer_requests: Option<io::PeerRequests>,
    args: &'a Args,
}

/// A SUBSCRIBE of the peer this client accepted.
struct Serving {
    request: PeerRequestId,
    /// The peer's request stream, as the readers name it.
    stream: u64,
    send: <WtConnection as Connection>::Send,
    track_alias: u64,
    /// Subgroup streams opened for the subscription, for `PUBLISH_DONE`.
    streams: u64,
}

/// Completes with the code of the `STOP_SENDING` the subscriber sent on the
/// stream of the subscription that is being served; never, when none is.
async fn stopped(serving: &mut Option<Serving>) -> Option<u32> {
    match serving {
        Some(serving) => match serving.send.stopped().await {
            xmoq_moqt_io::WriteError::Stopped(code) => code,
            // A lost connection is reported where the session is watched.
            xmoq_moqt_io::WriteError::Lost(_) => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
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
        let mut stream = match RequestStream::open(self.connection, &self.args.trace, label).await {
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
                self.stop_streams_of(track_alias);
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
                    UniEvent::SubgroupOpened { stream, header } => {
                        if self.belongs_to(header.track_alias, track_alias) {
                            self.open_streams.insert(stream, header.track_alias);
                        } else {
                            // §11.1, §11.4.2: a stream of an ended subscription,
                            // or of none, is not wanted.
                            report::ignored(header.track_alias);
                            self.stops.stop(stream, stream_reset_code::CANCELLED);
                        }
                    }
                    UniEvent::Object { header, object, .. } => {
                        if self.belongs_to(header.track_alias, track_alias) {
                            report::object(&header, &object);
                            received += 1;
                        }
                    }
                    UniEvent::SubgroupClosed { stream, header, end } => {
                        self.open_streams.remove(&stream);
                        self.session.on_data_stream_closed(header.track_alias);
                        match end {
                            StreamEnd::Fin => {}
                            StreamEnd::Aborted(code) => report::stream_aborted(stream, code),
                            // The same line as for a reset without a code:
                            // the drivers of interop runs read it.
                            StreamEnd::Lost => report::stream_aborted(stream, None),
                        }
                    }
                    UniEvent::TrackMalformed { header, error, .. } => {
                        let malformed = header.track_alias;
                        report::track_malformed(malformed, &error);
                        if malformed == track_alias {
                            // §2.4.2: the subscription is cancelled. The session
                            // is not at fault and is closed without an error.
                            self.session.cancel(request_id);
                            self.stop_streams_of(track_alias);
                            if let Some(stream) = stream.take() {
                                stream.cancel();
                            }
                            return Err(error.into());
                        }
                    }
                    other => bail!("misrouted event {other:?}"),
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
                            self.stop_streams_of(track_alias);
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
                self.stop_streams_of(track_alias);
                report::subscription_ended(why, received, wanted);
                return Ok(());
            }
        }

        // Enough Objects: the subscription is no longer wanted. Nothing happens
        // in the session when PUBLISH_DONE already ended the request.
        self.session.cancel(request_id);
        self.stop_streams_of(track_alias);
        if let Some(stream) = stream {
            stream.cancel();
        }
        Ok(())
    }

    /// Sends `STOP_SENDING` on the open subgroup streams of a subscription that
    /// is over (§10.11: "SHOULD send `STOP_SENDING` on all streams related to the
    /// subscription when it deletes subscription state").
    fn stop_streams_of(&mut self, track_alias: u64) {
        self.open_streams.retain(|stream, alias| {
            if *alias == track_alias {
                self.stops.stop(*stream, stream_reset_code::CANCELLED);
            }
            *alias != track_alias
        });
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
        tokio::time::sleep(self.args.publish_delay).await;

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

    /// Sends one Object as Object 0 of a new Group, on a new subgroup stream.
    async fn send_object(&self, track_alias: u64, group_id: u64, payload: &[u8]) -> Result<()> {
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias,
            group_id,
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
        let mut data = self.connection.open_uni().await?;
        let bytes = encoder.take_bytes();
        self.args
            .trace
            .bytes(Direction::Send, &format!("data#{group_id}"), &bytes);
        let failed = |error| anyhow!("could not write the subgroup stream: {error:?}");
        data.write_all(&bytes).await.map_err(failed)?;
        // §11.4.3: every Object of the Subgroup was written, so the stream ends
        // with a FIN.
        match tokio::time::timeout(DELIVERY_WAIT, data.finish_and_wait()).await {
            // A peer that does not want the stream (§11.4.1) has what it wants.
            Ok(Ok(())) => {}
            Ok(Err(xmoq_moqt_io::WriteError::Stopped(code))) => {
                report::stop_sending_received(&format!("data#{group_id}"), code);
            }
            Ok(Err(error)) => return Err(failed(error)),
            Err(_) => bail!("the peer did not take the subgroup stream in time"),
        }
        report::sent(group_id, payload.len());
        Ok(())
    }

    async fn publish(&mut self, payload: &[u8]) -> Result<()> {
        let (_namespace_stream, publish_request, mut publish_stream) = self.offer_track().await?;

        // The driver writes a line for every Object it wants sent, once the
        // subscribers are ready, and closes stdin when it is done; the session
        // has to stay open in between. The request stream is read all the while: the subscriber
        // may update the subscription on it, or cancel by closing it.
        let mut stdin = BufReader::new(tokio::io::stdin()).lines();
        let mut stream_count = 0;
        let status_code = loop {
            tokio::select! {
                line = stdin.next_line() => match line? {
                    Some(_) => {
                        // Each Object is a Group of its own, numbered from 0.
                        self.send_object(PUBLISH_TRACK_ALIAS, stream_count, payload)
                            .await?;
                        stream_count += 1;
                    }
                    None if stream_count == 0 => bail!("stdin closed before the send trigger"),
                    None => break publish_done_status::TRACK_ENDED,
                },
                // This client has no subgroup stream open between Objects, so a
                // subscriber that ends the subscription leaves none to reset
                // (§5.1.1).
                event = publish_stream.next_event_or_stopped() => match event? {
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
                    // §5.1: the subscriber ends the subscription with
                    // STOP_SENDING. One that only finishes its direction can
                    // send no REQUEST_UPDATE any more and has ended nothing.
                    RequestStreamEvent::Closed(RequestClose::Fin) => {
                        report::subscriber_finished_its_direction();
                    }
                    RequestStreamEvent::Closed(how) => {
                        if let RequestClose::Stopped(code) = how {
                            report::stop_sending_received("request", code);
                        }
                        self.session.cancel(publish_request);
                        publish_stream.cancel();
                        bail!("the subscriber ended the subscription: {how}");
                    }
                },
            }
        };

        // §10.11: PUBLISH_DONE goes out after the data streams are closed, and
        // the request stream is closed right after it. `send_object` closes its
        // data stream before it returns.
        let done = self.session.publish_done(
            publish_request,
            status_code,
            stream_count,
            ReasonPhrase::default(),
        )?;
        publish_stream.send(&done).await?;
        let answered = publish_stream
            .finish_and_await_close(PUBLISH_DONE_ANSWER_WAIT)
            .await?;
        // Whatever the peer did is reported and is not an error of this run:
        // the draft does not ask a subscriber to close its direction at all.
        report::publish_done_peer_ended(answered);
        Ok(())
    }

    /// Announces the namespace and publishes the track to the peer when it
    /// subscribes. One Object goes out for every line on stdin, as in
    /// [`Self::publish`]; a line that comes while nobody is subscribed is an
    /// error of the driver, which is to wait for `SERVING`.
    async fn serve(&mut self, payload: &[u8]) -> Result<()> {
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
        let mut peer_requests = self
            .peer_requests
            .take()
            .context("the readers were not started to serve requests")?;
        let track = self.args.full_track_name()?;
        let mut stdin = BufReader::new(tokio::io::stdin()).lines();
        let mut serving: Option<Serving> = None;
        // Each Object is a Group of its own, numbered from 0 across
        // subscriptions.
        let mut group_id = 0;
        loop {
            tokio::select! {
                line = stdin.next_line() => match line? {
                    Some(_) => {
                        let Some(serving) = &mut serving else {
                            bail!("asked to send while nobody is subscribed");
                        };
                        self.send_object(serving.track_alias, group_id, payload).await?;
                        group_id += 1;
                        serving.streams += 1;
                    }
                    None => break,
                },
                event = peer_requests.next_event() => {
                    self.on_peer_request_event(event?, &track, &mut serving).await?;
                }
                // §5.1: the subscriber ends the subscription with STOP_SENDING.
                // This client has no subgroup stream open between Objects, so
                // there is none to reset (§5.1.1).
                code = stopped(&mut serving) => {
                    report::stop_sending_received("request", code);
                    if let Some(ended) = serving.take() {
                        self.session.on_peer_request_closed(ended.request);
                        ended.send.reset(stream_reset_code::CANCELLED);
                        report::subscription_cancelled();
                    }
                }
            }
        }

        // The driver is done. §10.11: PUBLISH_DONE after the data streams are
        // closed, then a FIN; `send_object` closes its stream before it
        // returns.
        let Some(mut ended) = serving else {
            return Ok(());
        };
        let done = self.session.subscribe_done(
            ended.request,
            publish_done_status::TRACK_ENDED,
            ended.streams,
            ReasonPhrase::default(),
        )?;
        xmoq_moqt_io::send_message(&mut ended.send, &done, &self.args.trace, "served").await?;
        ended.send.finish();
        // The same wait as after a PUBLISH: see `PUBLISH_DONE_ANSWER_WAIT`.
        let peer_ended = async {
            loop {
                if let PeerRequestStreamEvent::Closed { stream, how } =
                    peer_requests.next_event().await?
                    && stream == ended.stream
                {
                    return Ok::<_, anyhow::Error>(how.is_fin());
                }
            }
        };
        let fin = match tokio::time::timeout(PUBLISH_DONE_ANSWER_WAIT, peer_ended).await {
            Ok(fin) => Some(fin?),
            Err(_) => None,
        };
        self.session.on_peer_request_closed(ended.request);
        report::publish_done_peer_ended_on_its_stream(fin);
        Ok(())
    }

    /// One event of a request stream the peer opened, in the serving mode.
    async fn on_peer_request_event(
        &mut self,
        event: PeerRequestStreamEvent<<WtConnection as Connection>::Send>,
        track: &FullTrackName,
        serving: &mut Option<Serving>,
    ) -> Result<()> {
        let wire = &self.args.trace;
        match event {
            PeerRequestStreamEvent::Opened {
                stream,
                mut send,
                first,
            } => {
                let label = format!("bi#{stream}");
                let (request, peer_request) = match self.session.on_peer_request_opened(first)? {
                    PeerRequestOpened::Request(request, peer_request) => (request, peer_request),
                    PeerRequestOpened::Refused { response } => {
                        xmoq_moqt_io::send_message(&mut send, &response, wire, &label).await?;
                        send.finish();
                        return Ok(());
                    }
                };
                let wanted = matches!(
                    &peer_request,
                    PeerRequest::Subscribe(subscribe) if subscribe.full_track_name == *track
                );
                if wanted && serving.is_none() {
                    let (track_alias, ok) =
                        self.session.accept_subscribe(request, vec![], vec![])?;
                    xmoq_moqt_io::send_message(&mut send, &ok, wire, &label).await?;
                    report::serving(track_alias);
                    *serving = Some(Serving {
                        request,
                        stream,
                        send,
                        track_alias,
                        streams: 0,
                    });
                    return Ok(());
                }
                // Another track, or a kind of request this client does not
                // serve. A second SUBSCRIBE for the track while one is served
                // does not get here: the session refuses it itself
                // (DUPLICATE_SUBSCRIPTION, §5.1).
                let error_code = match peer_request {
                    PeerRequest::Subscribe(_) => request_error_code::DOES_NOT_EXIST,
                    PeerRequest::Publish(_) | PeerRequest::PublishNamespace(_) => {
                        request_error_code::UNINTERESTED
                    }
                };
                let response =
                    self.session
                        .reject(request, error_code, 0, ReasonPhrase::default())?;
                xmoq_moqt_io::send_message(&mut send, &response, wire, &label).await?;
                send.finish();
            }
            PeerRequestStreamEvent::Message { stream, message } => {
                let Some(served) = serving.as_mut().filter(|served| served.stream == stream) else {
                    return Ok(());
                };
                let event = self
                    .session
                    .on_peer_request_message(served.request, message)?;
                // §10.9: this client applies no update, answers the first one
                // with REQUEST_ERROR, and then ends the subscription
                // (§10.9.1).
                if let PeerRequestEvent::UpdateRefused { response, .. } = event {
                    xmoq_moqt_io::send_message(&mut served.send, &response, wire, "served").await?;
                    report::update_refused();
                    if let Some(mut ended) = serving.take() {
                        let done = self.session.subscribe_done(
                            ended.request,
                            publish_done_status::UPDATE_FAILED,
                            ended.streams,
                            ReasonPhrase::default(),
                        )?;
                        xmoq_moqt_io::send_message(&mut ended.send, &done, wire, "served").await?;
                        ended.send.finish();
                    }
                }
            }
            PeerRequestStreamEvent::Closed { stream, how } => {
                // A subscriber that only finishes its direction has not ended
                // the subscription (§5.1); a reset has (§3.3.2).
                if !how.is_fin()
                    && serving
                        .as_ref()
                        .is_some_and(|served| served.stream == stream)
                    && let Some(ended) = serving.take()
                {
                    self.session.on_peer_request_closed(ended.request);
                    ended.send.reset(stream_reset_code::CANCELLED);
                    report::subscription_cancelled();
                }
            }
            PeerRequestStreamEvent::Failed(error) => return Err(error.into()),
            PeerRequestStreamEvent::ReaderPanicked => bail!("a stream reader failed"),
        }
        Ok(())
    }

    async fn run_mode(&mut self) -> Result<()> {
        match &self.args.mode {
            Mode::Subscribe { objects } => self.subscribe(*objects).await,
            Mode::Publish { payload } => self.publish(payload).await,
            Mode::PublishNamespace { payload } => self.serve(payload).await,
        }
    }
}

/// What has to stay until the session is closed, so the caller keeps it.
#[derive(Default)]
struct UntilClosed {
    /// This endpoint's control stream. §3.3: it stays open for the whole
    /// session; ending it earlier is a `PROTOCOL_VIOLATION` for the peer.
    control: Option<<WtConnection as Connection>::Send>,
    /// The readers of the peer's streams, its control stream among them.
    /// WHY kept: letting go of a stream that is not over tells the peer to
    /// stop sending on it, and the close of the session takes a moment to
    /// reach the peer. A relay that sees its control stream stopped first
    /// closes the session with `PROTOCOL_VIOLATION`.
    readers: Option<io::Readers>,
}

/// Runs the session until its mode is done. `kept` receives what has to
/// outlive this function.
async fn run_session(
    connection: &Arc<WtConnection>,
    kept: &mut UntilClosed,
    args: &Args,
) -> Result<()> {
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let stops = StreamStops::default();
    let serve = matches!(args.mode, Mode::PublishNamespace { .. });
    let (mut session_inbox, objects, peer_requests, readers) = io::start_readers(
        Arc::clone(connection),
        args.trace.clone(),
        stops.clone(),
        serve,
        args.answer_delay,
    );
    kept.readers = Some(readers);

    // §3.3: each peer opens one control stream that begins with SETUP.
    let control = kept.control.insert(connection.open_uni().await?);
    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    let setup = session.setup(vec![KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        implementation.as_bytes().to_vec(),
    )?])?;
    xmoq_moqt_io::send_message(control, &setup, &args.trace, "control").await?;

    let peer_setup = session.on_control_message(session_inbox.next_control_message().await?)?;
    report::setup_ok(&peer_setup);

    let mut client = Client {
        connection,
        session,
        objects,
        stops,
        open_streams: HashMap::new(),
        peer_requests,
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

    let connection = tokio::time::timeout_at(deadline, wt::connect(&args.url, args.cert_sha256))
        .await
        .map_err(|_| timed_out())??;
    let connection = Arc::new(connection);
    report::connected(&args.url);

    // WHY the timeout sits here and not around `run`: the close below has to
    // happen on every path, a timeout included.
    let mut kept = UntilClosed::default();
    let session = run_session(&connection, &mut kept, args);
    let result = match tokio::time::timeout_at(deadline, session).await {
        Ok(result) => result,
        Err(_) => Err(timed_out()),
    };
    connection.close(termination_code(&result), b"");
    // The close carries its code to the peer, which takes a moment; a process
    // that exits before that leaves the peer with a lost connection.
    let _ = tokio::time::timeout(CLOSE_GRACE, connection.closed()).await;
    drop(kept);
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
