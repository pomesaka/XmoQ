//! The requests of a viewer on its session: SUBSCRIBE, and the catalog of
//! MSF by a Joining FETCH.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::SubgroupObject;
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::kvp::Value;
use xmoq_moqt::message::{ControlMessage, FetchRange, GoAway, RequestError, request_error_code};
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter, answer_wait_ms};
use xmoq_moqt::session::{ControlEvent, RequestEvent, RequestId, Session};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, RequestReader, RequestStreamEvent, SendStream, StreamStops, Trace,
    UniEvent, send_message,
};
use xmoq_msf::CatalogTrack;

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

/// The name of the catalog track (MSF §5).
const CATALOG: &str = "catalog";
/// How long the catalog has to come, as the watching page waits.
const CATALOG_WAIT: Duration = Duration::from_secs(5);
/// The most a catalog may be.
const CATALOG_LIMIT: usize = 1024 * 1024;

/// A subscription: its request stream, kept open for as long as the track
/// is wanted.
pub struct Subscription {
    pub request: RequestId,
    pub track_alias: u64,
    pub reader: RequestReader<Recv>,
    send: Send,
}

impl Subscription {
    /// Gives the subscription up on the stream: a reset of this side and
    /// `STOP_SENDING` for the other, with `CANCELLED` (§3.3.2). The session's
    /// half is `Session::cancel`.
    pub fn cancel(self) {
        self.send.reset(stream_reset_code::CANCELLED);
        self.reader.stop(stream_reset_code::CANCELLED);
    }
}

/// What has to stay open for as long as the session after the catalog was
/// read: the subscription to the catalog track and the request stream of
/// the FETCH.
pub struct CatalogKept {
    _subscription: Subscription,
    _fetch: Send,
}

/// What the requests of a session are sent with.
pub struct Requests<'a> {
    pub connection: &'a WtConnection,
    pub session: &'a mut Session,
    pub namespace: &'a TrackNamespace,
    /// The AUTHORIZATION TOKEN every request carries, where there is one.
    pub token: Option<&'a [u8]>,
    /// `--answer-wait-ms`.
    pub answer_wait_ms: u64,
    /// `--open-wait-ms`.
    pub open_wait_ms: u64,
    /// `--rendezvous-timeout-ms`: on every SUBSCRIBE.
    pub rendezvous_ms: Option<u64>,
}

/// Opens the stream of a request, within the time the relay has to allow
/// one more. Nothing was sent when it fails, and what the session has of
/// the request is the caller's to let go of.
///
/// WHY a time: a relay that allows no stream would hold the viewer at
/// "connecting" for good. The page and the Rust client give a request the
/// same time (the row "A request that waits for a stream" of
/// docs/moqt-implementation-notes.md).
async fn open_stream(
    connection: &WtConnection,
    open_wait_ms: u64,
    what: &str,
) -> Result<(Send, Recv)> {
    let wait = Duration::from_millis(open_wait_ms);
    let Ok(opened) = tokio::time::timeout(wait, connection.open_bi()).await else {
        bail!("the relay allowed no stream for {what} within {open_wait_ms} ms");
    };
    Ok(opened?)
}

/// A SUBSCRIBE the session made and that is not sent yet.
pub struct Asked {
    name: String,
    request: RequestId,
    message: ControlMessage,
    /// How long its answer is waited for.
    wait_ms: u64,
}

/// A SUBSCRIBE that was sent, and the first thing that came on its stream,
/// or nothing within its time.
pub struct Sent {
    send: Send,
    reader: RequestReader<Recv>,
    answer: Option<Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>>,
}

/// Why a SUBSCRIBE gave no subscription.
pub enum NotSubscribed {
    /// The relay refused it.
    Refused(RequestError),
    /// It was not sent, not answered in time, or answered with something
    /// else.
    Failed(anyhow::Error),
}

/// A SUBSCRIBE the relay refused with `GOING_AWAY` (§10.4): it has sent
/// GOAWAY, which may come after the refusal on the control stream.
#[derive(Debug)]
pub struct GoingAway(pub String);

impl std::fmt::Display for GoingAway {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "SUBSCRIBE of {} was refused: GOING_AWAY", self.0)
    }
}

impl std::error::Error for GoingAway {}

/// The relay's GOAWAY, read before the viewer came to play.
#[derive(Debug)]
pub struct GoneAway(pub GoAway);

impl std::fmt::Display for GoneAway {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the relay sent GOAWAY")
    }
}

impl std::error::Error for GoneAway {}

/// Sends `asked` and waits for its answer. Needs no session, so that what
/// is played goes on meanwhile; [`settle`] hands the answer to the session.
pub async fn send_subscribe(
    connection: &WtConnection,
    open_wait_ms: u64,
    asked: &Asked,
) -> Result<Sent> {
    let (mut send, recv) = open_stream(connection, open_wait_ms, &asked.name).await?;
    let trace = Trace::default();
    send_message(&mut send, &asked.message, &trace, &asked.name).await?;
    let mut reader = RequestReader::new(recv, &trace, &asked.name);
    // WHY from here, and the request alone given up: the row "A request
    // the peer does not answer" of docs/moqt-implementation-notes.md,
    // which the page and the Rust client go by as well. What the viewer
    // makes of a track it did not get is its caller's.
    let answer = tokio::time::timeout(Duration::from_millis(asked.wait_ms), reader.next_event())
        .await
        .ok();
    Ok(Sent {
        send,
        reader,
        answer,
    })
}

/// Hands what came of a SUBSCRIBE to the session: a subscription, or why
/// there is none. A request that got no answer, or could not be sent, is
/// let go of in the session.
pub fn settle(
    session: &mut Session,
    asked: &Asked,
    sent: Result<Sent>,
) -> Result<Subscription, NotSubscribed> {
    let (request, name) = (asked.request, &asked.name);
    let failed = |error: anyhow::Error| NotSubscribed::Failed(error);
    let mut sent = match sent {
        Ok(sent) => sent,
        Err(error) => {
            session.cancel(request);
            return Err(failed(error));
        }
    };
    let Some(answer) = sent.answer.take() else {
        session.cancel(request);
        sent.send.reset(stream_reset_code::CANCELLED);
        sent.reader.stop(stream_reset_code::CANCELLED);
        return Err(failed(anyhow!(
            "the relay did not answer the SUBSCRIBE of {name} within {} ms",
            asked.wait_ms
        )));
    };
    let RequestStreamEvent::Message(response) = answer.map_err(|error| failed(error.into()))?
    else {
        return Err(failed(anyhow!(
            "the relay closed the SUBSCRIBE of {name} without an answer"
        )));
    };
    match session
        .on_request_message(request, response)
        .map_err(|error| failed(error.into()))?
    {
        RequestEvent::SubscribeEstablished { track_alias, .. } => Ok(Subscription {
            request,
            track_alias,
            reader: sent.reader,
            send: sent.send,
        }),
        RequestEvent::Rejected(error) => Err(NotSubscribed::Refused(error)),
        other => Err(failed(anyhow!(
            "SUBSCRIBE of {name} was answered with {other:?}"
        ))),
    }
}

impl Requests<'_> {
    /// Subscribes to a track of the namespace. With `timeout_ms` the
    /// SUBSCRIBE carries a `SUBGROUP_DELIVERY_TIMEOUT` (§10.2.3): the
    /// publisher gives up a Subgroup that is not delivered that long after
    /// its last Object (§8).
    pub async fn subscribe(&mut self, name: &str, timeout_ms: Option<u64>) -> Result<Subscription> {
        let asked = self.ask(name, timeout_ms)?;
        let sent = send_subscribe(self.connection, self.open_wait_ms, &asked).await;
        settle(self.session, &asked, sent).map_err(|not| match not {
            NotSubscribed::Refused(error) if error.error_code == request_error_code::GOING_AWAY => {
                GoingAway(name.to_string()).into()
            }
            NotSubscribed::Refused(error) => {
                anyhow!("SUBSCRIBE of {name} was refused: {error:?}")
            }
            NotSubscribed::Failed(error) => error,
        })
    }

    /// The SUBSCRIBE for a track of the namespace, made by the session and
    /// not sent yet ([`send_subscribe`]).
    pub fn ask(&mut self, name: &str, timeout_ms: Option<u64>) -> Result<Asked> {
        let full_name = FullTrackName::new(self.namespace.clone(), name.as_bytes().to_vec())?;
        let mut parameters = vec![
            Parameter::Forward(true),
            Parameter::SubscriberPriority(128),
            Parameter::GroupOrder(GroupOrder::Ascending),
        ];
        if let Some(token) = self.token {
            parameters.push(Parameter::AuthorizationToken(token.to_vec()));
        }
        if let Some(timeout_ms) = timeout_ms {
            parameters.push(Parameter::SubgroupDeliveryTimeout(timeout_ms));
        }
        if let Some(wait_ms) = self.rendezvous_ms {
            parameters.push(Parameter::RendezvousTimeout(wait_ms));
        }
        let wait_ms = answer_wait_ms(self.answer_wait_ms, &parameters);
        let (request, message) = self.session.subscribe(full_name, parameters)?;
        Ok(Asked {
            name: name.to_string(),
            request,
            message,
            wait_ms,
        })
    }
    /// Reads the catalog of the namespace: its tracks.
    ///
    /// WHY a Joining FETCH: the catalog is one Object that was published
    /// before the viewer came, so a SUBSCRIBE alone does not bring it
    /// (§10.12.2: the latest Group of the track the SUBSCRIBE is for), as
    /// `followCatalog` of `@xmoq/msf` does.
    pub async fn catalog(
        &mut self,
        events: &mut mpsc::Receiver<UniEvent>,
        stops: &StreamStops,
    ) -> Result<(Vec<CatalogTrack>, CatalogKept)> {
        let subscription = self.subscribe(CATALOG, None).await?;
        let range = FetchRange::RelativeJoining {
            joining_request_id: subscription.request.value(),
            joining_start: 0,
        };
        let parameters = self
            .token
            .map(|token| Parameter::AuthorizationToken(token.to_vec()))
            .into_iter()
            .collect();
        let (fetch, message) = self.session.fetch(range, parameters)?;
        let (mut send, recv) =
            open_stream(self.connection, self.open_wait_ms, "the catalog's FETCH").await?;
        let trace = Trace::default();
        send_message(&mut send, &message, &trace, "fetch").await?;
        let mut answers = Some(RequestReader::new(recv, &trace, "fetch"));
        let session = &mut *self.session;

        let payload = async {
            loop {
                tokio::select! {
                    answer = async {
                        match answers.as_mut() {
                            Some(answers) => answers.next_event().await,
                            None => std::future::pending().await,
                        }
                    } => match answer? {
                        // FETCH_OK, or a refusal: then the catalog can
                        // still come on the subscription.
                        RequestStreamEvent::Message(message) => {
                            session.on_request_message(fetch, message)?;
                        }
                        RequestStreamEvent::Closed(_) => answers = None,
                    },
                    event = events.recv() => match event.ok_or_else(|| anyhow!("the connection is gone"))? {
                        UniEvent::Object { header, object, .. }
                            if header.track_alias == subscription.track_alias =>
                        {
                            return Ok::<_, anyhow::Error>(object.payload);
                        }
                        UniEvent::FetchOpened { stream, header } => {
                            session.on_fetch_stream(header.request_id)?;
                            let Some(mut body) = stops.take_fetch_body(stream) else { continue };
                            let mut decoder = FetchStreamDecoder::new(CATALOG_LIMIT, GroupOrder::Ascending);
                            // The readers took the header off; the decoder wants to see one.
                            decoder.push(&FetchStreamEncoder::new(header, GroupOrder::Ascending).take_bytes())?;
                            // The last Object of the Group is the catalog as it is now.
                            let mut latest = None;
                            loop {
                                while let Some(event) = decoder.next_event()? {
                                    if let FetchEvent::Object(object) = event {
                                        latest = Some(object.payload);
                                    }
                                }
                                match body.next().await {
                                    FetchChunk::Bytes(bytes) => decoder.push(&bytes)?,
                                    _ => break,
                                }
                            }
                            if let Some(payload) = latest {
                                return Ok(payload);
                            }
                        }
                        UniEvent::SubgroupClosed { header, .. } => {
                            session.on_data_stream_closed(header.track_alias);
                        }
                        UniEvent::Failed(error) => return Err(error.into()),
                        // §10.4. A GOAWAY that breaks its rules is an error
                        // of the session, as in `play`.
                        UniEvent::Control(message) => {
                            if let ControlEvent::GoAway(go_away) = session.on_control_message(message)? {
                                return Err(GoneAway(go_away).into());
                            }
                        }
                        _ => {}
                    },
                }
            }
        };
        // The catalog's 5 seconds begin when its SUBSCRIBE was answered,
        // which a relay that holds it for a broadcast to come has done.
        let payload = tokio::time::timeout(CATALOG_WAIT, payload)
            .await
            .map_err(|_| anyhow!("no catalog came within 5 s"))??;
        let tracks = xmoq_msf::read_catalog(&payload).context("the catalog")?;
        Ok((
            tracks,
            CatalogKept {
                _subscription: subscription,
                _fetch: send,
            },
        ))
    }
}

/// The Timestamp of LOC of an Object, in microseconds. `None` for an
/// Object without one, with one that is invalid, or with a Timescale of
/// its own, which is not handled: the demo has none (wall-clock
/// microseconds).
pub fn timestamp_of(object: &SubgroupObject) -> Option<u64> {
    let timing = loc::read_timing(object.properties.iter().map(|pair| {
        let value = match pair.value() {
            Value::Varint(value) => Some(*value),
            Value::Bytes(_) => None,
        };
        (pair.ty(), value)
    }))
    .ok()??;
    timing.timescale.is_none().then_some(timing.timestamp)
}
