//! The chat on `MoQ` tracks, on the session of the broadcast
//! (`xmoq_watch::chat_track` decides): the viewer announces its namespace
//! and publishes `said` to the gatherer that subscribes to it, and takes the
//! room's `lines` by SUBSCRIBE and its latest lines by a Joining FETCH.
//! Nothing of it ends the session: what fails is said, and the broadcast
//! plays on.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_chat_tracks::limits::TEXT_BYTES;
use xmoq_chat_tracks::names::{LINES, SAID, person_namespace};
use xmoq_chat_tracks::payloads::{Line, Said, read_line};
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::message::{ControlMessage, FetchRange};
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{
    PeerRequest, PeerRequestId, PeerRequestOpened, RequestEvent, RequestId, Session,
};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, PeerRequestStreamEvent, RequestReader, RequestStreamEvent, SendStream,
    StreamStops, Trace, send_message,
};
use xmoq_watch::again::AskAgain;
use xmoq_watch::args::Chat;
use xmoq_watch::chat_track::{Say, Saying, Shown, fetch_start, shown};

use crate::session::{Asked, NotSubscribed, Requests, Sent, Subscription, send_subscribe, settle};

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

/// The most bytes of the stream of the Joining FETCH: 16 lines of
/// [`TEXT_BYTES`] and their JSON, with room.
const FETCH_LIMIT: usize = 64 * 1024;

/// The most an Object of `lines` may be: a line of [`TEXT_BYTES`] and its
/// JSON, with room. The page's chat has the same (`@xmoq/moqt`'s
/// `maxObjectPayloadLength`). **Provisional.**
const LINES_PAYLOAD_MOST: usize = 64 * 1024;

/// How long a stream for a line of `said` may wait to be opened. WHY not
/// waited for longer: a line said late is worth less than the next, as an
/// Object of a live track (the Rust client's `SUBGROUP_OPEN_WAIT`).
const SAID_OPEN_WAIT: Duration = Duration::from_secs(2);

/// How long the relay is asked to hold the SUBSCRIBE to `lines` for the
/// gatherer to publish it: the most the own relay holds
/// (`MAX_RENDEZVOUS_MS`). WHY held: the gatherer publishes `lines` of a room
/// once a participant has come, and the viewer that comes first would
/// otherwise be refused before its own announcement reached the gatherer.
const LINES_RENDEZVOUS_MS: u64 = 30_000;

/// A SUBSCRIBE to `lines` on its way, and what came of it.
type Answer =
    std::pin::Pin<Box<dyn std::future::Future<Output = (Asked, Result<Sent>)> + std::marker::Send>>;

/// What the chat on tracks has to be told of, from the loop of the session.
pub enum Came {
    /// A line of the FETCH, by its number.
    Fetched(u64, Vec<u8>),
    /// The FETCH's stream ended, or no stream comes.
    FetchEnded,
    /// What came on the FETCH's request stream.
    FetchAnswer(Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>),
    /// The time to ask for `lines` (again) has come.
    AskNow,
    /// What came of the SUBSCRIBE to `lines`, boxed: it is the largest by far.
    Settled(Box<(Asked, Result<Sent>)>),
    /// What came on the stream of the subscription to `lines`.
    Lines(Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>),
}

/// What the loop is to do with what came: a line to show, or a line for
/// `--log`.
pub enum Out {
    Show(String),
    Note(String),
}

/// The chat on tracks of one session.
pub struct ChatTracks {
    connection: Arc<WtConnection>,
    /// Where the limit of `lines` on its Objects is given.
    stops: StreamStops,
    /// `xmoq/chat/<room>`.
    room: TrackNamespace,
    token: Option<Vec<u8>>,
    answer_wait_ms: u64,
    open_wait_ms: u64,
    /// `xmoq/chat/<room>/<name>`, `said`.
    said: FullTrackName,
    saying: Saying,
    shown: Shown<Line>,
    /// The subscription to `lines`, while there is one.
    lines: Option<Subscription>,
    asking: Option<Answer>,
    ask_at: Option<tokio::time::Instant>,
    again: AskAgain,
    /// The Joining FETCH, while its answer may still come.
    fetch: Option<(RequestId, RequestReader<Recv>)>,
    /// The request streams that stay open while the session lasts: the
    /// `PUBLISH_NAMESPACE`'s and those of each `FETCH`.
    kept: Vec<Send>,
    /// The half of the `PUBLISH_NAMESPACE`'s stream the relay writes to.
    /// WHY kept, though nothing more is read: letting go of it stops the
    /// stream, and the relay takes the announcement as withdrawn (the
    /// gatherer is told `NAMESPACE_DONE` at once; seen in the first run).
    _announced: RequestReader<Recv>,
    /// The gatherer's SUBSCRIBE to `said`: its stream, request, Track Alias,
    /// and the half of its stream that stays open.
    subscriber: Option<(u64, PeerRequestId, u64, Send)>,
    fetched: (mpsc::Sender<Came>, mpsc::Receiver<Came>),
}

/// Opens the stream of a request and sends its message, within `wait_ms`.
async fn send_request(
    connection: &WtConnection,
    wait_ms: u64,
    message: &ControlMessage,
    label: &str,
) -> Result<(Send, RequestReader<Recv>)> {
    let wait = Duration::from_millis(wait_ms);
    let Ok(opened) = tokio::time::timeout(wait, connection.open_bi()).await else {
        bail!("the relay allowed no stream for {label} within {wait_ms} ms");
    };
    let (mut send, recv) = opened?;
    let trace = Trace::default();
    send_message(&mut send, message, &trace, label).await?;
    Ok((send, RequestReader::new(recv, &trace, label)))
}

/// Waits for `item` where there is one, and for ever where there is none.
async fn on<T>(item: Option<impl std::future::Future<Output = T>>) -> T {
    match item {
        Some(future) => future.await,
        None => std::future::pending().await,
    }
}

impl ChatTracks {
    /// Announces the viewer's namespace and waits for the answer, so that a
    /// relay that refuses it is said at once. `lines` is asked for from the
    /// loop ([`Came::AskNow`]), where the broadcast plays meanwhile.
    pub async fn begin(
        requests: Requests<'_>,
        connection: &Arc<WtConnection>,
        stops: &StreamStops,
        chat: &Chat,
        room: TrackNamespace,
    ) -> Result<Self> {
        let Requests {
            session,
            token,
            answer_wait_ms,
            open_wait_ms,
            ..
        } = requests;
        let tokens: Vec<Parameter> = token
            .map(|token| Parameter::AuthorizationToken(token.to_vec()))
            .into_iter()
            .collect();
        let mine = person_namespace(&chat.room, &chat.name)?;
        let said = FullTrackName::new(mine.clone(), SAID.as_bytes().to_vec())?;
        let (announce, message) = session.publish_namespace(mine, tokens)?;
        let (announce_send, mut answers) =
            send_request(connection, open_wait_ms, &message, "publish-namespace").await?;
        let answer =
            tokio::time::timeout(Duration::from_millis(answer_wait_ms), answers.next_event())
                .await
                .map_err(|_| {
                    anyhow!("no answer to PUBLISH_NAMESPACE within {answer_wait_ms} ms")
                })?;
        let RequestStreamEvent::Message(answer) = answer? else {
            bail!("the PUBLISH_NAMESPACE was closed without an answer");
        };
        match session.on_request_message(announce, answer)? {
            RequestEvent::PublishNamespaceEstablished => {}
            other => bail!("the PUBLISH_NAMESPACE was answered with {other:?}"),
        }
        Ok(Self {
            connection: Arc::clone(connection),
            stops: stops.clone(),
            room,
            token: token.map(<[u8]>::to_vec),
            answer_wait_ms,
            open_wait_ms,
            said,
            saying: Saying::default(),
            shown: Shown::fetching(),
            lines: None,
            asking: None,
            ask_at: Some(tokio::time::Instant::now()),
            again: AskAgain::default(),
            fetch: None,
            kept: vec![announce_send],
            _announced: answers,
            subscriber: None,
            fetched: mpsc::channel(32),
        })
    }

    /// The Track Alias of `lines`, while it is subscribed to.
    pub fn lines_alias(&self) -> Option<u64> {
        self.lines.as_ref().map(|lines| lines.track_alias)
    }

    /// The Request ID of the Joining FETCH, while it is open.
    pub fn fetch_request_id(&self) -> Option<u64> {
        self.fetch.as_ref().map(|(fetch, _)| fetch.value())
    }

    /// What comes next.
    pub async fn next(&mut self) -> Came {
        let Self {
            fetched,
            fetch,
            asking,
            ask_at,
            lines,
            ..
        } = self;
        tokio::select! {
            came = fetched.1.recv() => came.unwrap_or(Came::FetchEnded),
            answer = on(fetch.as_mut().map(|(_, answers)| answers.next_event())) => Came::FetchAnswer(answer),
            settled = on(asking.as_mut()) => Came::Settled(Box::new(settled)),
            () = on(ask_at.map(tokio::time::sleep_until)) => Came::AskNow,
            event = on(lines.as_mut().map(|lines| lines.reader.next_event())) => Came::Lines(event),
        }
    }

    /// What came, and what the loop is to do with it.
    pub async fn on(&mut self, session: &mut Session, came: Came) -> Vec<Out> {
        match came {
            Came::Fetched(number, payload) => self.fetched(number, &payload).into_iter().collect(),
            Came::FetchEnded => self.fetch_ended(),
            Came::FetchAnswer(answer) => match self.on_fetch_answer(session, answer) {
                Ok(shown) => shown,
                Err(error) => {
                    let mut out = vec![Out::Note(format!("CHAT_TRACKS_FETCH_FAILED {error:#}"))];
                    out.extend(self.fetch_ended());
                    out
                }
            },
            Came::AskNow => {
                self.ask_at = None;
                self.ask(session)
            }
            Came::Settled(settled) => {
                self.asking = None;
                let (asked, sent) = *settled;
                self.settled(session, &asked, sent).await
            }
            Came::Lines(event) => self.on_lines(session, event),
        }
    }

    /// Sends the SUBSCRIBE to `lines`, held by the relay for the gatherer.
    fn ask(&mut self, session: &mut Session) -> Vec<Out> {
        let mut requests = Requests {
            connection: &self.connection,
            session,
            namespace: &self.room,
            token: self.token.as_deref(),
            answer_wait_ms: self.answer_wait_ms,
            open_wait_ms: self.open_wait_ms,
            rendezvous_ms: Some(LINES_RENDEZVOUS_MS),
        };
        match requests.ask(LINES, None) {
            Ok(asked) => {
                let connection = Arc::clone(&self.connection);
                let open_wait_ms = self.open_wait_ms;
                self.asking = Some(Box::pin(async move {
                    let sent = send_subscribe(&connection, open_wait_ms, &asked).await;
                    (asked, sent)
                }));
                Vec::new()
            }
            Err(error) => vec![Out::Note(format!("CHAT_TRACKS_LINES_GIVEN_UP {error:#}"))],
        }
    }

    /// What came of the SUBSCRIBE to `lines`: a subscription and its Joining
    /// FETCH, or a pause before it is asked for again.
    async fn settled(
        &mut self,
        session: &mut Session,
        asked: &Asked,
        sent: Result<Sent>,
    ) -> Vec<Out> {
        let subscription = match settle(session, asked, sent) {
            Ok(subscription) => subscription,
            Err(NotSubscribed::Refused(error)) => {
                let pause = self.again.after_refusal(error.retry_wait_ms());
                self.ask_in(pause);
                return vec![Out::Note(format!(
                    "CHAT_TRACKS_LINES_REFUSED error_code={:#x}; asking again in {pause} ms",
                    error.error_code
                ))];
            }
            Err(NotSubscribed::Failed(error)) => {
                let pause = self.again.after_end();
                self.ask_in(pause);
                return vec![Out::Note(format!(
                    "CHAT_TRACKS_LINES_FAILED {error:#}; asking again in {pause} ms"
                ))];
            }
        };
        self.again.subscribed();
        let mut out = vec![Out::Note(format!(
            "CHAT_TRACKS_LINES track_alias={}",
            subscription.track_alias
        ))];
        self.stops
            .limit_payload(subscription.track_alias, LINES_PAYLOAD_MOST);
        let start = fetch_start(subscription.largest.map(|largest| largest.group));
        let joining_request_id = subscription.request.value();
        self.lines = Some(subscription);
        self.shown.refetching();
        // No line yet: nothing to fetch.
        let Some(joining_start) = start else {
            out.extend(self.fetch_ended());
            return out;
        };
        let range = FetchRange::AbsoluteJoining {
            joining_request_id,
            joining_start,
        };
        let tokens = self
            .token
            .iter()
            .map(|token| Parameter::AuthorizationToken(token.clone()))
            .collect();
        let sent = match session.fetch(range, tokens) {
            Ok((fetch, message)) => {
                send_request(&self.connection, self.open_wait_ms, &message, "fetch")
                    .await
                    .map(|(request_send, answers)| (fetch, request_send, answers))
            }
            Err(error) => Err(error.into()),
        };
        match sent {
            Ok((fetch, request_send, answers)) => {
                self.kept.push(request_send);
                self.fetch = Some((fetch, answers));
            }
            Err(error) => {
                out.push(Out::Note(format!("CHAT_TRACKS_FETCH_FAILED {error:#}")));
                out.extend(self.fetch_ended());
            }
        }
        out
    }

    /// What came on the stream of the subscription to `lines`. The
    /// subscription that ends (the gatherer ends `lines` of a room nobody
    /// is in with `TRACK_ENDED`) is asked for again, as the sound's is.
    fn on_lines(
        &mut self,
        session: &mut Session,
        event: Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>,
    ) -> Vec<Out> {
        let Some(lines) = self.lines.as_ref() else {
            return Vec::new();
        };
        let request = lines.request;
        let how = match event {
            Ok(RequestStreamEvent::Message(message)) => {
                match session.on_request_message(request, message) {
                    Ok(RequestEvent::SubscriptionDone(done)) => {
                        format!("PUBLISH_DONE status_code={:#x}", done.status_code)
                    }
                    Ok(other) => {
                        return vec![Out::Note(format!("CHAT_TRACKS_LINES_SAID {other:?}"))];
                    }
                    Err(error) => format!("{error:?}"),
                }
            }
            Ok(RequestStreamEvent::Closed(closed)) => {
                session.cancel(request);
                format!("closed {closed:?}")
            }
            Err(error) => {
                session.cancel(request);
                format!("{error}")
            }
        };
        if let Some(lines) = self.lines.take() {
            self.stops.unlimit_payload(lines.track_alias);
            lines.cancel();
        }
        let pause = self.again.after_end();
        self.ask_in(pause);
        vec![Out::Note(format!(
            "CHAT_TRACKS_LINES_ENDED {how}; asking again in {pause} ms"
        ))]
    }

    fn ask_in(&mut self, pause_ms: u64) {
        self.ask_at = Some(tokio::time::Instant::now() + Duration::from_millis(pause_ms));
    }

    /// Reads the stream of the FETCH on a task of its own; its lines come
    /// back by [`Self::next`].
    pub fn read_fetch(
        &self,
        stops: &StreamStops,
        stream: u64,
        header: xmoq_moqt::fetch::FetchHeader,
    ) {
        let Some(mut body) = stops.take_fetch_body(stream) else {
            return;
        };
        let to = self.fetched.0.clone();
        tokio::spawn(async move {
            let mut decoder = FetchStreamDecoder::new(FETCH_LIMIT, GroupOrder::Ascending);
            // The readers took the header off; the decoder wants to see one.
            if decoder
                .push(&FetchStreamEncoder::new(header, GroupOrder::Ascending).take_bytes())
                .is_ok()
            {
                'read: loop {
                    loop {
                        match decoder.next_event() {
                            Ok(Some(FetchEvent::Object(object))) => {
                                let _ = to
                                    .send(Came::Fetched(object.group_id, object.payload))
                                    .await;
                            }
                            Ok(Some(_)) => {}
                            Ok(None) => break,
                            Err(_) => break 'read,
                        }
                    }
                    match body.next().await {
                        FetchChunk::Bytes(bytes) => {
                            if decoder.push(&bytes).is_err() {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
            }
            let _ = to.send(Came::FetchEnded).await;
        });
    }

    /// What came on the FETCH's request stream: its answer. A refusal (a
    /// room without lines yet is `INVALID_RANGE`, §10.12.3) ends the FETCH,
    /// and the lines that waited are shown. Returns the lines to show.
    fn on_fetch_answer(
        &mut self,
        session: &mut Session,
        answer: Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>,
    ) -> Result<Vec<Out>> {
        let Some((fetch, _)) = self.fetch.as_ref() else {
            return Ok(Vec::new());
        };
        let fetch = *fetch;
        match answer {
            Ok(RequestStreamEvent::Message(message)) => {
                match session.on_request_message(fetch, message)? {
                    RequestEvent::Rejected(_) => Ok(self.fetch_ended()),
                    _ => Ok(Vec::new()),
                }
            }
            // Its stream brings the lines, or brought them.
            Ok(RequestStreamEvent::Closed(_)) => {
                self.fetch = None;
                Ok(Vec::new())
            }
            Err(error) => {
                self.fetch = None;
                Err(error.into())
            }
        }
    }

    /// The FETCH is over: the lines that waited, shown.
    fn fetch_ended(&mut self) -> Vec<Out> {
        self.fetch = None;
        self.shown
            .fetch_ended()
            .into_iter()
            .map(|(number, line)| Out::Show(shown(number, &line.name, &line.text)))
            .collect()
    }

    /// A line of the FETCH: the line to show, if it is shown.
    fn fetched(&mut self, number: u64, payload: &[u8]) -> Option<Out> {
        let line = read_line(number, payload).ok()?;
        let (number, line) = self.shown.fetched(number, line)?;
        Some(Out::Show(shown(number, &line.name, &line.text)))
    }

    /// An Object of `lines`: the line to show, if it is shown.
    pub fn live(&mut self, number: u64, payload: &[u8]) -> Option<String> {
        let line = read_line(number, payload).ok()?;
        let (number, line) = self.shown.live(number, line)?;
        Some(shown(number, &line.name, &line.text))
    }

    /// A line the viewer says. `false` where it is not sent: too long, or
    /// too many wait.
    pub fn say(&mut self, connection: &Arc<WtConnection>, text: String) -> bool {
        if text.len() > TEXT_BYTES {
            return false;
        }
        match self.saying.say(text) {
            Say::Now(group, text) => {
                self.write(connection, group, &text);
                true
            }
            Say::Held => true,
            Say::Dropped => false,
        }
    }

    /// Writes a line on `said` as its own Group, on a task: one stream,
    /// Object 0, a FIN.
    fn write(&self, connection: &Arc<WtConnection>, group: u64, text: &str) {
        let Some((_, _, track_alias, _)) = self.subscriber.as_ref() else {
            return;
        };
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias: *track_alias,
            group_id: group,
            subgroup_id: SubgroupId::Explicit(0),
            publisher_priority: Some(128),
            properties: false,
            // The line is the Group's one Object.
            end_of_group: true,
            first_object: true,
        });
        let sent_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            });
        let object = SubgroupObject {
            object_id: 0,
            properties: vec![],
            status: ObjectStatus::Normal,
            payload: Said {
                text: text.to_string(),
                sent_at,
            }
            .payload(),
        };
        if encoder.object(&object).is_err() {
            return;
        }
        let bytes = encoder.take_bytes();
        let connection = Arc::clone(connection);
        tokio::spawn(async move {
            let Ok(Ok(mut stream)) =
                tokio::time::timeout(SAID_OPEN_WAIT, connection.open_uni()).await
            else {
                return;
            };
            if stream.write_all(&bytes).await.is_ok() {
                stream.finish();
            }
        });
    }

    /// A request stream the peer opened. The gatherer's SUBSCRIBE to `said`
    /// is taken, and what is said waits no more; anything else is handed
    /// back, for the viewer to refuse.
    pub async fn on_peer_request(
        &mut self,
        session: &mut Session,
        connection: &Arc<WtConnection>,
        event: PeerRequestStreamEvent<Send>,
    ) -> Result<Option<PeerRequestStreamEvent<Send>>> {
        match event {
            PeerRequestStreamEvent::Opened {
                stream,
                mut send,
                first: ControlMessage::Subscribe(subscribe),
            } if subscribe.full_track_name == self.said && self.subscriber.is_none() => {
                let opened =
                    session.on_peer_request_opened(ControlMessage::Subscribe(subscribe))?;
                let request = match opened {
                    PeerRequestOpened::Request(request, PeerRequest::Subscribe(_)) => request,
                    PeerRequestOpened::Refused { response, .. } => {
                        let _ =
                            send_message(&mut send, &response, &Trace::default(), "request").await;
                        send.finish();
                        return Ok(None);
                    }
                    PeerRequestOpened::Request(..) => {
                        bail!("a SUBSCRIBE was read as another request")
                    }
                };
                let (track_alias, ok) = session.accept_subscribe(request, vec![], vec![])?;
                send_message(&mut send, &ok, &Trace::default(), "said").await?;
                self.subscriber = Some((stream, request, track_alias, send));
                for (group, text) in self.saying.subscribed() {
                    self.write(connection, group, &text);
                }
                Ok(None)
            }
            PeerRequestStreamEvent::Message { stream, message }
                if self
                    .subscriber
                    .as_ref()
                    .is_some_and(|(said, ..)| *said == stream) =>
            {
                if let Some((_, request, ..)) = &self.subscriber {
                    session.on_peer_request_message(*request, message)?;
                }
                Ok(None)
            }
            PeerRequestStreamEvent::Closed { stream, .. }
                if self
                    .subscriber
                    .as_ref()
                    .is_some_and(|(said, ..)| *said == stream) =>
            {
                if let Some((_, request, ..)) = self.subscriber.take() {
                    session.on_peer_request_closed(request);
                }
                self.saying.unsubscribed();
                Ok(None)
            }
            other => Ok(Some(other)),
        }
    }
}
