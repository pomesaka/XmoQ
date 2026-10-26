//! The requests of a viewer on its session: SUBSCRIBE, and the catalog of
//! MSF by a Joining FETCH.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::RequestFault;
use xmoq_moqt::data::SubgroupObject;
use xmoq_moqt::early::{Arrived, EarlyObjects, held_bytes};
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::immutable::{Of, searched_integers};
use xmoq_moqt::message::{ControlMessage, FetchRange, GoAway, RequestError, request_error_code};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter, answer_wait_ms};
use xmoq_moqt::session::{ControlEvent, PeerRequestId, RequestEvent, RequestId, Session};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, PeerRequestStreamEvent, RequestReader, RequestStreamEvent, SendStream,
    StreamEnd, StreamStops, Trace, UniEvent, client, send_message,
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
pub(crate) const MOST_READ_AHEAD: usize = 256;

/// The events of the session's unidirectional streams, with those read
/// ahead while an answer was waited for, which come first.
pub struct Events {
    received: mpsc::Receiver<UniEvent>,
    ahead: VecDeque<UniEvent>,
    /// The readers are gone: nothing more comes.
    ended: bool,
    /// Objects read before the `SUBSCRIBE_OK` that gives their alias
    /// (`xmoq_moqt::early`).
    early: EarlyObjects<UniEvent>,
    /// The Track Alias of each subgroup stream that is open, by its
    /// stream; no more than the readers take at once
    /// (`StreamConfig::max_uni_streams`).
    open: HashMap<u64, u64>,
}

impl Events {
    /// The events of `received`, from the session's readers.
    #[must_use]
    pub fn new(received: mpsc::Receiver<UniEvent>) -> Self {
        Self {
            received,
            ahead: VecDeque::new(),
            ended: false,
            early: EarlyObjects::default(),
            open: HashMap::new(),
        }
    }

    /// Stops every subgroup stream of `track_alias` that is open, with
    /// `CANCELLED`: what the session ended for a malformed track
    /// (`Session::cancel_malformed`) leaves its other streams to the caller
    /// (§10.11), and the readers stopped the one that broke alone.
    pub fn stop_streams_of(&mut self, track_alias: u64, stops: &StreamStops) {
        self.open.retain(|stream, alias| {
            if *alias == track_alias {
                stops.stop(*stream, stream_reset_code::CANCELLED);
            }
            *alias != track_alias
        });
    }

    /// The next event, as [`Self::recv`], for a caller that looks at the
    /// Track Alias of an Object: an Object of an alias the session does not
    /// know yet is held while a `SUBSCRIBE` of it waits for its answer, and
    /// comes once the session took the `SUBSCRIBE_OK`; its stream is stopped
    /// where no `SUBSCRIBE` waits, or where too much is held
    /// (`xmoq_moqt::early`). `None` when the readers are gone.
    ///
    /// WHY here and not in the readers: what an alias is, is the session's,
    /// which the caller's loop holds; the `SUBSCRIBE_OK` comes on a stream of
    /// its own, and can be read after the first Objects of its track.
    /// Called anew each time round the caller's loop, it first gives what
    /// the session took meanwhile let go of.
    pub async fn recv_sorted(
        &mut self,
        session: &Session,
        stops: &StreamStops,
    ) -> Option<UniEvent> {
        self.settle(session, stops);
        loop {
            let event = self.recv().await?;
            if let Some(event) = self.sort(session, stops, event) {
                return Some(event);
            }
        }
    }

    fn sort(
        &mut self,
        session: &Session,
        stops: &StreamStops,
        event: UniEvent,
    ) -> Option<UniEvent> {
        match event {
            UniEvent::Object {
                stream,
                ref header,
                ref object,
            } => {
                let (alias, bytes) = (header.track_alias, held_bytes(object));
                match self.early.arrive(session, stream, alias, bytes, event) {
                    Arrived::Deliver(event) => Some(event),
                    Arrived::Held => None,
                    Arrived::Stop { stream, code } => {
                        stops.stop(stream, code);
                        None
                    }
                }
            }
            UniEvent::SubgroupOpened { stream, ref header } => {
                self.open.insert(stream, header.track_alias);
                Some(event)
            }
            UniEvent::SubgroupClosed { stream, end, .. } => {
                self.open.remove(&stream);
                self.early
                    .stream_closed(stream, !matches!(end, StreamEnd::Fin));
                Some(event)
            }
            event => Some(event),
        }
    }

    /// Gives the Objects held for an alias the session knows now to
    /// [`Self::recv`], before anything else, in the order they came; and
    /// stops the streams of those no `SUBSCRIBE` can give any more.
    fn settle(&mut self, session: &Session, stops: &StreamStops) {
        if self.early.is_empty() {
            return;
        }
        let settled = self.early.settle(session);
        for (stream, code) in settled.stopped {
            stops.stop(stream, code);
        }
        self.put_back(settled.released.into());
    }

    /// The next event; `None` when the readers are gone.
    pub async fn recv(&mut self) -> Option<UniEvent> {
        if let Some(event) = self.ahead.pop_front() {
            return Some(event);
        }
        self.received.recv().await
    }

    /// Puts events that were read, and not taken, before those read ahead,
    /// in the order they came.
    pub(crate) fn put_back(&mut self, mut kept: VecDeque<UniEvent>) {
        kept.append(&mut self.ahead);
        self.ahead = kept;
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
        self.cancel_with(stream_reset_code::CANCELLED);
    }

    /// [`Self::cancel`] with `code`: `MALFORMED_TRACK` for a track that
    /// broke the object model (§2.4.2).
    pub fn cancel_with(self, code: u32) {
        self.send.reset(code);
        self.reader.stop(code);
    }
}

/// What has to stay open for as long as the session after the catalog was
/// read: the subscription to the catalog track and the request stream of
/// the FETCH.
pub struct CatalogKept {
    subscription: Option<Subscription>,
    fetch: Option<(RequestId, Send)>,
}

impl CatalogKept {
    /// Resets the stream of `request` with `MALFORMED_TRACK`, where it is
    /// the catalog's SUBSCRIBE or FETCH, which the session ended for a
    /// malformed track (`Session::cancel_malformed`); whether it was. The
    /// catalog is not asked for again: neither viewer follows a catalog that
    /// changes during a session.
    pub fn malformed(&mut self, request: RequestId) -> bool {
        if self
            .subscription
            .as_ref()
            .is_some_and(|subscription| subscription.request == request)
        {
            if let Some(subscription) = self.subscription.take() {
                subscription.cancel_with(stream_reset_code::MALFORMED_TRACK);
            }
            return true;
        }
        if self
            .fetch
            .as_ref()
            .is_some_and(|(fetch, _)| *fetch == request)
        {
            if let Some((_, send)) = self.fetch.take() {
                send.reset(stream_reset_code::MALFORMED_TRACK);
            }
            return true;
        }
        false
    }
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
    /// Its answer broke the protocol: the session is to be closed with its
    /// code, as for any message of the peer that does (`RequestFault::Peer`).
    Peer(RequestFault),
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
    let (track_alias, largest) = answer_of(session, request, name, response)?;
    Ok(Subscription {
        request,
        track_alias,
        largest,
        reader: sent.reader,
        send: sent.send,
    })
}

/// What the session makes of `response`, the answer to the SUBSCRIBE
/// `request` of `name`: the Track Alias of the subscription and the
/// `LARGEST_OBJECT` of its `SUBSCRIBE_OK`, or why there is none.
fn answer_of(
    session: &mut Session,
    request: RequestId,
    name: &str,
    response: ControlMessage,
) -> Result<(u64, Option<Location>), NotSubscribed> {
    match session
        .on_request_message(request, response)
        .map_err(|fault| match fault {
            RequestFault::Peer(_) => NotSubscribed::Peer(fault),
            RequestFault::Usage(_) => NotSubscribed::Failed(fault.into()),
        })? {
        RequestEvent::SubscribeEstablished {
            track_alias,
            parameters,
            ..
        } => Ok((
            track_alias,
            parameters.iter().find_map(|parameter| match parameter {
                Parameter::LargestObject(location) => Some(*location),
                _ => None,
            }),
        )),
        RequestEvent::Rejected(error) => Err(NotSubscribed::Refused(error)),
        other => Err(NotSubscribed::Failed(anyhow!(
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
            NotSubscribed::Peer(fault) => fault.into(),
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
        // WHY the other events are kept, and not let go of: the catalog of a
        // participant of a meeting is read while the session goes on, and
        // what came meanwhile (a line of the chat, a piece of speech, an
        // Object of a video) is the loop's. WHY reading stops when as many
        // are kept as `Events` reads ahead, and none is dropped: a line or a
        // piece that is dropped makes its number skip without a word, and an
        // Object of a video the rest of its Group. Flow control holds the
        // relay back meanwhile, and the catalog has its 5 seconds.
        let mut kept = VecDeque::new();

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
                    event = events.recv(), if kept.len() < MOST_READ_AHEAD => match event.ok_or_else(|| anyhow!("the connection is gone"))? {
                        UniEvent::Object { header, object, .. }
                            if header.track_alias == subscription.track_alias =>
                        {
                            return Ok::<_, anyhow::Error>(object.payload);
                        }
                        UniEvent::FetchOpened { stream, header } if header.request_id == fetch.value() => {
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
                            // Over once it was answered too: the session
                            // holds it no more (`cancel_malformed`).
                            session.on_fetch_stream_closed(fetch);
                            if let Some(payload) = latest {
                                return Ok(payload);
                            }
                        }
                        // Of another track, it is kept with its Objects, in
                        // their order.
                        UniEvent::SubgroupClosed { stream, header, .. }
                            if header.track_alias == subscription.track_alias =>
                        {
                            events.open.remove(&stream);
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
                        other => kept.push_back(other),
                    },
                }
            }
        };
        // The catalog's 5 seconds begin when its SUBSCRIBE was answered,
        // which a relay that holds it for a broadcast to come has done.
        let payload = tokio::time::timeout(CATALOG_WAIT, payload).await;
        events.put_back(kept);
        let payload = payload.map_err(|_| anyhow!("no catalog came within 5 s"))??;
        let tracks = xmoq_msf::read_catalog(&payload).context("the catalog")?;
        Ok((
            tracks,
            CatalogKept {
                subscription: Some(subscription),
                fetch: Some((fetch, send)),
            },
        ))
    }
}

impl client::UniEvents for Events {
    fn next_event(
        &mut self,
    ) -> impl std::future::Future<Output = Option<UniEvent>> + std::marker::Send {
        self.recv()
    }
}

/// Opens the control stream, sends SETUP with `token` where there is one
/// and `implementation` as `MOQT_IMPLEMENTATION`, and hands the relay's
/// SETUP to the session: `client::setup` of `xmoq-moqt-io`, which keeps the
/// control stream in `control` and has the errors.
///
/// # Errors
///
/// Those of `client::setup`; `client::termination_code` gives the code to
/// close with.
pub async fn setup(
    connection: &WtConnection,
    session: &mut Session,
    events: &mut Events,
    control: &mut Option<Send>,
    token: Option<&[u8]>,
    implementation: &str,
    setup_wait_ms: u64,
) -> Result<()> {
    let options = client::setup_options(token, implementation)?;
    let wait = Duration::from_millis(setup_wait_ms);
    client::setup(connection, session, events, control, options, wait).await?;
    Ok(())
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
/// (`client::termination_code` of `xmoq-moqt-io`), and a reader that
/// panicked.
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
    loc::wall_clock_timestamp(searched_integers(&object.properties, Of::Object))
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
    async fn what_was_put_back_comes_first_in_the_order_it_came() {
        // (kept while the catalog was waited for; read ahead before that;
        // still in the queue; the order they are read in)
        type Case = (
            &'static [u64],
            &'static [u64],
            &'static [u64],
            &'static [u64],
        );
        let cases: [Case; 4] = [
            (&[1, 2], &[3, 4], &[5], &[1, 2, 3, 4, 5]),
            (&[1, 2], &[], &[3], &[1, 2, 3]),
            (&[], &[1], &[2], &[1, 2]),
            (&[], &[], &[], &[]),
        ];
        for (kept, ahead, queued, expected) in cases {
            let (queue, received) = mpsc::channel(8);
            let mut events = Events::new(received);
            events.ahead = ahead.iter().map(|id| control(*id)).collect();
            for id in queued {
                queue.send(control(*id)).await.unwrap();
            }
            drop(queue);
            events.put_back(kept.iter().map(|id| control(*id)).collect());
            let mut read = Vec::new();
            while let Some(event) = events.recv().await {
                let UniEvent::Control(ControlMessage::UnsupportedRequest { request_id, .. }) =
                    event
                else {
                    panic!("{event:?}");
                };
                read.push(request_id);
            }
            assert_eq!(read, expected, "{kept:?} {ahead:?} {queued:?}");
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

    #[test]
    fn the_answer_to_a_subscribe_makes_a_subscription_a_refusal_or_an_end_of_the_session() {
        use xmoq_moqt::message::{ReasonPhrase, RequestOk, SubscribeOk};

        /// What the answer is to give.
        #[derive(Debug)]
        enum Expected {
            /// A subscription of this Track Alias.
            Subscribed(u64),
            /// A refusal of this Error Code.
            Refused(u64),
            /// No subscription, and the session goes on.
            Failed,
            /// The session is to be closed.
            Peer,
        }
        let cases = [
            (
                "SUBSCRIBE_OK",
                ControlMessage::SubscribeOk(SubscribeOk {
                    track_alias: 3,
                    parameters: vec![],
                    track_properties: vec![],
                }),
                Expected::Subscribed(3),
            ),
            (
                "REQUEST_ERROR",
                ControlMessage::RequestError(RequestError {
                    error_code: 0x10,
                    retry_interval: 0,
                    error_reason: ReasonPhrase::default(),
                    redirect: None,
                }),
                Expected::Refused(0x10),
            ),
            (
                "a GOAWAY before the answer",
                ControlMessage::GoAway(GoAway {
                    new_session_uri: vec![],
                    timeout_ms: 0,
                    request_id: None,
                }),
                Expected::Failed,
            ),
            // A REQUEST_OK answers no SUBSCRIBE.
            (
                "REQUEST_OK",
                ControlMessage::RequestOk(RequestOk::default()),
                Expected::Peer,
            ),
        ];
        for (name, response, expected) in cases {
            let mut session = Session::new(
                xmoq_moqt::session::Role::Client,
                xmoq_moqt::session::Transport::WebTransport,
            );
            let track = FullTrackName::new(
                TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap(),
                b"audio".to_vec(),
            )
            .unwrap();
            let (request, _) = session.subscribe(track, vec![]).unwrap();
            match (
                answer_of(&mut session, request, "audio", response),
                expected,
            ) {
                (Ok((alias, None)), Expected::Subscribed(expected)) => {
                    assert_eq!(alias, expected, "{name}");
                }
                (Err(NotSubscribed::Refused(error)), Expected::Refused(code)) => {
                    assert_eq!(error.error_code, code, "{name}");
                }
                (Err(NotSubscribed::Failed(_)), Expected::Failed)
                | (Err(NotSubscribed::Peer(RequestFault::Peer(_))), Expected::Peer) => {}
                (Ok(answer), expected) => panic!("{name}: {answer:?}, not {expected:?}"),
                (Err(_), expected) => panic!("{name}: another error, not {expected:?}"),
            }
        }
    }
}
