//! The requests of a viewer on its session: SUBSCRIBE, and the catalog of
//! MSF by a Joining FETCH.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::SubgroupObject;
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::immutable::{Of, searched_properties};
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::message::{
    ControlMessage, FetchRange, GoAway, RequestError, request_error_code, setup_option,
};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter, answer_wait_ms};
use xmoq_moqt::session::{ControlEvent, PeerRequestId, RequestEvent, RequestId, Session};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, PeerRequestStreamEvent, RequestReader, RequestStreamEvent, SendStream,
    StreamStops, Trace, UniEvent, send_message,
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

/// Events of the session's unidirectional streams read ahead of their turn
/// that are kept, while an answer is waited for ([`Events::while_waiting`]).
/// **Provisional.** Beyond it reading pauses, and flow control holds the
/// relay back.
const MOST_READ_AHEAD: usize = 256;

/// The events of the session's unidirectional streams, with those read
/// ahead while an answer was waited for, which come first.
pub struct Events {
    received: mpsc::Receiver<UniEvent>,
    ahead: VecDeque<UniEvent>,
    /// The readers are gone: nothing more comes.
    ended: bool,
}

impl Events {
    /// The events of `received`, from the session's readers.
    #[must_use]
    pub fn new(received: mpsc::Receiver<UniEvent>) -> Self {
        Self {
            received,
            ahead: VecDeque::new(),
            ended: false,
        }
    }

    /// The next event; `None` when the readers are gone.
    pub async fn recv(&mut self) -> Option<UniEvent> {
        if let Some(event) = self.ahead.pop_front() {
            return Some(event);
        }
        self.received.recv().await
    }

    /// Waits for `answer` and reads the events meanwhile: one that fails the
    /// session (a datagram of an unknown type, a stream that breaks the
    /// protocol) ends the wait with its error, and the rest are kept for
    /// [`Self::recv`].
    ///
    /// WHY: the answer to a request may take its time (10 s, and more with
    /// a `RENDEZVOUS_TIMEOUT`), and a session that the peer broke meanwhile is
    /// to be closed for it then, with its code, and not when the wait is
    /// over. WHY the rest is kept and not let go of: an Object of a track
    /// can be read before the answer that subscribes to it.
    ///
    /// # Errors
    ///
    /// The error of an event that fails the session.
    pub async fn while_waiting<T>(&mut self, answer: impl Future<Output = T>) -> Result<T> {
        // Boxed: the requests it waits for are large futures, and the
        // viewer's own future would grow by each.
        let mut answer = Box::pin(answer);
        loop {
            let reading = !self.ended && self.ahead.len() < MOST_READ_AHEAD;
            tokio::select! {
                biased;
                event = self.received.recv(), if reading => match event {
                    Some(UniEvent::Failed(error)) => return Err(error.into()),
                    Some(event) => self.ahead.push_back(event),
                    None => self.ended = true,
                },
                answered = &mut answer => return Ok(answered),
            }
        }
    }
}

/// A subscription: its request stream, kept open for as long as the track
/// is wanted.
pub struct Subscription {
    /// The session's handle of the SUBSCRIBE.
    pub request: RequestId,
    /// What its Objects come with.
    pub track_alias: u64,
    /// The `LARGEST_OBJECT` of its `SUBSCRIBE_OK`, where it had one.
    pub largest: Option<Location>,
    /// What the relay says on its stream after the answer: `PUBLISH_DONE`.
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
    /// Where the streams of the requests are opened.
    pub connection: &'a WtConnection,
    /// What makes the requests and reads their answers.
    pub session: &'a mut Session,
    /// The namespace of the tracks asked for.
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
///
/// # Errors
///
/// A stream the relay does not allow in time, and one that breaks before
/// the SUBSCRIBE is written.
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
///
/// # Errors
///
/// [`NotSubscribed`].
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
        RequestEvent::SubscribeEstablished {
            track_alias,
            parameters,
            ..
        } => Ok(Subscription {
            request,
            track_alias,
            largest: parameters.iter().find_map(|parameter| match parameter {
                Parameter::LargestObject(location) => Some(*location),
                _ => None,
            }),
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
    ///
    /// # Errors
    ///
    /// A SUBSCRIBE that is refused ([`GoingAway`] for `GOING_AWAY`), not
    /// sent, or not answered in time.
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
    ///
    /// # Errors
    ///
    /// A name that makes no Full Track Name, and a session that can make no
    /// more requests.
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
    ///
    /// # Errors
    ///
    /// As [`Self::subscribe`]; a catalog that does not come within 5 s or
    /// cannot be read; a session the peer broke meanwhile; [`GoneAway`].
    pub async fn catalog(
        &mut self,
        events: &mut Events,
        stops: &StreamStops,
    ) -> Result<(Vec<CatalogTrack>, CatalogKept)> {
        let subscription = events
            .while_waiting(self.subscribe(CATALOG, None))
            .await??;
        // The catalog's Objects are held to its limit from here on, and not
        // to the 1 MiB undecoded of every subgroup stream alone. It stays for
        // the session, as the subscription does.
        stops.limit_payload(subscription.track_alias, CATALOG_LIMIT);
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

/// The relay sent no SETUP in so many milliseconds.
#[derive(Debug)]
pub struct NoSetupInTime(pub u64);

impl std::fmt::Display for NoSetupInTime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the relay sent no SETUP within {} ms", self.0)
    }
}

impl std::error::Error for NoSetupInTime {}

/// Opens the control stream, sends SETUP with `token` where there is one
/// and `implementation` as `MOQT_IMPLEMENTATION`, and hands the relay's
/// SETUP to the session. The control stream goes into `control` as soon as
/// it is open, also where what follows fails: it is to stay open until the
/// session is closed, since a relay that sees it end closes the session
/// with `PROTOCOL_VIOLATION` first (§3.3).
///
/// # Errors
///
/// A stream that does not open, a SETUP that cannot be sent, the error of
/// an event that fails the session before the relay's SETUP (closed with its
/// code, as a datagram of an unknown type that comes first has it, §11),
/// something else first, and [`NoSetupInTime`].
pub async fn setup(
    connection: &WtConnection,
    session: &mut Session,
    events: &mut Events,
    control: &mut Option<Send>,
    token: Option<&[u8]>,
    implementation: &str,
    setup_wait_ms: u64,
) -> Result<()> {
    let control = control.insert(connection.open_uni().await?);
    // In ascending order of their types: the token (0x03), then the
    // implementation (0x07).
    let mut options = Vec::new();
    if let Some(token) = token {
        options.push(KeyValuePair::bytes(
            setup_option::AUTHORIZATION_TOKEN,
            token.to_vec(),
        )?);
    }
    options.push(KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        implementation.as_bytes().to_vec(),
    )?);
    let setup = session.setup(options)?;
    send_message(control, &setup, &Trace::default(), "control").await?;
    match tokio::time::timeout(Duration::from_millis(setup_wait_ms), events.recv()).await {
        Ok(Some(UniEvent::Control(message))) => {
            session.on_control_message(message)?;
        }
        Ok(Some(UniEvent::Failed(error))) => return Err(error.into()),
        Ok(other) => bail!("no SETUP from the relay: {other:?}"),
        Err(_) => return Err(NoSetupInTime(setup_wait_ms).into()),
    }
    Ok(())
}

/// The code to close the session with after `error` (§3.5):
/// `CONTROL_MESSAGE_TIMEOUT` for a SETUP that did not come, as `@xmoq/moqt`
/// has it, the one a protocol error of the relay asks for (a GOAWAY that
/// breaks the rules of §10.4 among them), and `NO_ERROR` for everything
/// else, also a request that is given up alone.
#[must_use]
pub fn termination_code(error: &anyhow::Error) -> u32 {
    if error.is::<NoSetupInTime>() {
        return 0x11;
    }
    // WHY the chain: `context` wraps the cause.
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<xmoq_moqt::Error>())
        .and_then(xmoq_moqt::Error::termination_code)
        .map_or(0, xmoq_moqt::TerminationCode::code)
}

/// What the peer does on a request stream it opened, where none is served:
/// each is refused (`Session::refuse_peer_request`: `NOT_SUPPORTED` and a
/// FIN, §3.3.2), as the Rust client does in its modes that serve none. What
/// still comes on a refused stream goes to the session, which checks the
/// Request ID of a `REQUEST_UPDATE` among it (§10.1).
///
/// # Errors
///
/// An error of the session, to be closed with its code
/// ([`termination_code`]), and a reader that panicked.
pub async fn refuse_peer_request<S: std::hash::BuildHasher>(
    session: &mut Session,
    refused: &mut HashMap<u64, PeerRequestId, S>,
    event: PeerRequestStreamEvent<Send>,
) -> Result<()> {
    match event {
        PeerRequestStreamEvent::Opened {
            stream,
            mut send,
            first,
        } => {
            let (response, request) = session.refuse_peer_request(first)?;
            if let Some(request) = request {
                refused.insert(stream, request);
            }
            // An error means the peer is gone or stopped the stream.
            let _ = send_message(&mut send, &response, &Trace::default(), "request").await;
            send.finish();
        }
        PeerRequestStreamEvent::Message { stream, message } => {
            // None where the session keeps nothing of it (`MAX_PEER_REQUESTS`).
            if let Some(request) = refused.get(&stream) {
                session.on_peer_request_message(*request, message)?;
            }
        }
        PeerRequestStreamEvent::Closed { stream, .. } => {
            if let Some(request) = refused.remove(&stream) {
                session.on_peer_request_closed(request);
            }
        }
        PeerRequestStreamEvent::Failed(error) => return Err(error.into()),
        PeerRequestStreamEvent::ReaderPanicked => bail!("a reader of a request stream panicked"),
    }
    Ok(())
}

/// Where a GOAWAY says to go: its New Session URI, or `None` for the URL of
/// the session (§10.4: "If the URI is zero bytes long, the current URI is
/// reused instead").
#[must_use]
pub fn new_url(go_away: &GoAway) -> Option<String> {
    (!go_away.new_session_uri.is_empty())
        .then(|| String::from_utf8_lossy(&go_away.new_session_uri).into_owned())
}

/// The Timestamp of LOC of an Object, in microseconds. `None` for an
/// Object without one, with one that is invalid, or with a Timescale of
/// its own, which is not handled: the demo has none (wall-clock
/// microseconds).
#[must_use]
pub fn timestamp_of(object: &SubgroupObject) -> Option<u64> {
    let timing = loc::read_timing(
        searched_properties(&object.properties, Of::Object)
            .iter()
            .map(|pair| {
                let value = match pair.value() {
                    Value::Varint(value) => Some(*value),
                    Value::Bytes(_) => None,
                };
                (pair.ty(), value)
            }),
    )
    .ok()??;
    timing.timescale.is_none().then_some(timing.timestamp)
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::message::ControlMessage;

    use super::*;

    fn control(id: u64) -> UniEvent {
        UniEvent::Control(ControlMessage::UnsupportedRequest {
            message_type: 0x7f,
            request_id: id,
        })
    }

    #[tokio::test]
    async fn an_error_of_the_session_ends_the_wait_for_an_answer() {
        let (queue, received) = mpsc::channel(8);
        let mut events = Events::new(received);
        queue.send(control(1)).await.unwrap();
        queue
            .send(UniEvent::Failed(xmoq_moqt::Error::ProtocolViolation(
                "an unknown datagram type",
            )))
            .await
            .unwrap();
        // An answer that never comes: the error ends the wait.
        let waited = events.while_waiting(std::future::pending::<()>()).await;
        let error = waited.unwrap_err();
        assert_eq!(
            error.downcast_ref::<xmoq_moqt::Error>(),
            Some(&xmoq_moqt::Error::ProtocolViolation(
                "an unknown datagram type"
            ))
        );
        // What came before it is kept, in its order.
        assert_eq!(events.recv().await, Some(control(1)));
    }

    #[tokio::test]
    async fn what_comes_while_an_answer_is_waited_for_is_read_after_it() {
        let (queue, received) = mpsc::channel(8);
        let mut events = Events::new(received);
        queue.send(control(1)).await.unwrap();
        queue.send(control(2)).await.unwrap();
        let (answer, answered) = tokio::sync::oneshot::channel();
        let waiting = events.while_waiting(answered);
        // The answer comes once the two were read.
        let send = async {
            tokio::task::yield_now().await;
            answer.send(7).unwrap();
        };
        let (waited, ()) = tokio::join!(waiting, send);
        assert_eq!(waited.unwrap().unwrap(), 7);
        queue.send(control(3)).await.unwrap();
        for id in 1..=3 {
            assert_eq!(events.recv().await, Some(control(id)), "{id}");
        }
    }

    #[tokio::test]
    async fn no_more_than_so_many_are_read_ahead() {
        let (queue, received) = mpsc::channel(MOST_READ_AHEAD + 8);
        let mut events = Events::new(received);
        for id in 0..=u64::try_from(MOST_READ_AHEAD).unwrap() {
            queue.send(control(id)).await.unwrap();
        }
        let waited = tokio::time::timeout(
            Duration::from_millis(50),
            events.while_waiting(std::future::pending::<()>()),
        )
        .await;
        assert!(waited.is_err());
        // The one beyond the limit waits in the queue.
        assert_eq!(events.ahead.len(), MOST_READ_AHEAD);
        assert_eq!(events.received.len(), 1);
    }
}
