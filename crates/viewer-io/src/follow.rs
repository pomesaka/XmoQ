//! Following a track of one item a Group, numbered by its Group ID without
//! a gap: the room's `lines` of the chat, and a transcript's `speech`. The
//! track is taken by SUBSCRIBE, with a `RENDEZVOUS_TIMEOUT` where its
//! publisher may come later, and its latest items by an Absolute Joining
//! FETCH; each item is given once, in the order of its number, and what was
//! missed while the subscription was away is fetched when it is made again
//! (`xmoq_viewer::chat_track` decides). A subscription that is refused or
//! ends is asked for again by the rule of `xmoq-again`.
//!
//! WHY one follower for both: the gatherer and the transcriber publish their
//! tracks alike (one item a Group, from 0, and a FETCH answered from what
//! they keep), and a reader wants the same of either.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use tokio::sync::mpsc;
use xmoq_again::AskAgain;
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::message::{ControlMessage, FetchRange};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{RequestEvent, RequestId, Session};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, RequestReader, RequestStreamEvent, StreamStops, Trace, send_message,
};
use xmoq_viewer::chat_track::{LinesGap, Shown, history_to_fetch};

use crate::session::{Asked, NotSubscribed, Requests, Sent, Subscription, send_subscribe, settle};

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

/// A SUBSCRIBE on its way, and what came of it.
type Answer =
    std::pin::Pin<Box<dyn std::future::Future<Output = (Asked, Result<Sent>)> + std::marker::Send>>;

/// What the follower has to be told of, from the loop of the session.
pub enum Came {
    /// An item of the FETCH, by its number.
    Fetched(u64, Vec<u8>),
    /// The FETCH's stream ended, or no stream comes.
    FetchEnded,
    /// What came on the FETCH's request stream.
    FetchAnswer(Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>),
    /// The time to ask for the track (again) has come.
    AskNow,
    /// What came of the SUBSCRIBE, boxed: it is the largest by far.
    Settled(Box<(Asked, Result<Sent>)>),
    /// What came on the stream of the subscription.
    Answer(Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>),
}

/// What the loop is to do with what came.
pub enum Out<T> {
    /// An item, each once, in the order of its number.
    Item(T),
    /// Items that were not fetched, told before the items after them.
    Gap(LinesGap),
    /// A line for the program's log, about the following itself.
    Note(String),
}

/// The track followed, and how.
pub struct Track<T> {
    /// Its namespace.
    pub namespace: TrackNamespace,
    /// Its name.
    pub name: &'static str,
    /// The `RENDEZVOUS_TIMEOUT` of its SUBSCRIBE, where its publisher may
    /// come later.
    pub rendezvous_ms: Option<u64>,
    /// The most an Object of the track may be.
    pub payload_most: usize,
    /// The most bytes of the stream of a FETCH.
    pub fetch_most: usize,
    /// How many of the items missed are fetched when the subscription is
    /// made again (`history_to_fetch`).
    pub catch_up: u64,
    /// What the notes begin with: `<note>` for the subscription,
    /// `<note>_...` for what becomes of it, and `<fetch_note>_...` for the
    /// FETCH and a numbering anew.
    pub note: &'static str,
    /// See [`Track::note`].
    pub fetch_note: &'static str,
    /// Reads the payload of the item of a Group; `None` for one that is no
    /// item, which is passed over.
    pub read: fn(u64, &[u8]) -> Option<T>,
}

/// What the requests are sent with.
pub struct Asking {
    /// Where their streams are opened.
    pub connection: Arc<WtConnection>,
    /// Where the limit on the track's Objects is given, and the FETCH's
    /// stream is taken.
    pub stops: StreamStops,
    /// The AUTHORIZATION TOKEN of the SUBSCRIBE and the FETCH.
    pub token: Option<Vec<u8>>,
    /// `--answer-wait-ms`.
    pub answer_wait_ms: u64,
    /// `--open-wait-ms`.
    pub open_wait_ms: u64,
}

/// One track followed on one session.
pub struct Follower<T> {
    track: Track<T>,
    asking_with: Asking,
    shown: Shown<T>,
    /// The subscription, while there is one.
    subscription: Option<Subscription>,
    asking: Option<Answer>,
    ask_at: Option<tokio::time::Instant>,
    again: AskAgain,
    /// The Joining FETCH, while its answer may still come.
    fetch: Option<(RequestId, RequestReader<Recv>)>,
    /// The request streams of each `FETCH`, which stay open while the
    /// session lasts.
    kept: Vec<Send>,
    fetched: (mpsc::Sender<Came>, mpsc::Receiver<Came>),
}

/// Opens the stream of a request and sends its message, within `wait_ms`.
///
/// # Errors
///
/// A stream the relay does not allow in time, and one that breaks before
/// the message is written.
pub async fn send_request(
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

impl<T> Follower<T> {
    /// A follower that asks for the track at once, from the loop
    /// ([`Came::AskNow`]). `had` is the newest item given on the sessions
    /// before, so that what was missed between them is fetched.
    #[must_use]
    pub fn new(track: Track<T>, asking_with: Asking, had: Option<u64>) -> Self {
        Self {
            track,
            asking_with,
            shown: Shown::fetching_after(had),
            subscription: None,
            asking: None,
            ask_at: Some(tokio::time::Instant::now()),
            again: AskAgain::default(),
            fetch: None,
            kept: Vec::new(),
            fetched: mpsc::channel(32),
        }
    }

    /// The number of the newest item given, if any.
    pub fn last_shown(&self) -> Option<u64> {
        self.shown.last()
    }

    /// The Track Alias of the track, while it is subscribed to.
    pub fn track_alias(&self) -> Option<u64> {
        self.subscription
            .as_ref()
            .map(|subscription| subscription.track_alias)
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
            subscription,
            ..
        } = self;
        tokio::select! {
            came = fetched.1.recv() => came.unwrap_or(Came::FetchEnded),
            answer = on(fetch.as_mut().map(|(_, answers)| answers.next_event())) => Came::FetchAnswer(answer),
            settled = on(asking.as_mut()) => Came::Settled(Box::new(settled)),
            () = on(ask_at.map(tokio::time::sleep_until)) => Came::AskNow,
            event = on(subscription.as_mut().map(|subscription| subscription.reader.next_event())) => Came::Answer(event),
        }
    }

    /// What came, and what the loop is to do with it.
    pub async fn on(&mut self, session: &mut Session, came: Came) -> Vec<Out<T>> {
        match came {
            Came::Fetched(number, payload) => self.fetched(number, &payload).into_iter().collect(),
            Came::FetchEnded => self.fetch_ended(),
            Came::FetchAnswer(answer) => match self.on_fetch_answer(session, answer) {
                Ok(shown) => shown,
                Err(error) => {
                    let mut out = vec![Out::Note(format!(
                        "{}_FETCH_FAILED {error:#}",
                        self.track.fetch_note
                    ))];
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
            Came::Answer(event) => self.on_answer(session, event),
        }
    }

    /// Sends the SUBSCRIBE.
    fn ask(&mut self, session: &mut Session) -> Vec<Out<T>> {
        let mut requests = Requests {
            connection: &self.asking_with.connection,
            session,
            namespace: &self.track.namespace,
            token: self.asking_with.token.as_deref(),
            answer_wait_ms: self.asking_with.answer_wait_ms,
            open_wait_ms: self.asking_with.open_wait_ms,
            rendezvous_ms: self.track.rendezvous_ms,
        };
        match requests.ask(self.track.name, None) {
            Ok(asked) => {
                let connection = Arc::clone(&self.asking_with.connection);
                let open_wait_ms = self.asking_with.open_wait_ms;
                self.asking = Some(Box::pin(async move {
                    let sent = send_subscribe(&connection, open_wait_ms, &asked).await;
                    (asked, sent)
                }));
                Vec::new()
            }
            Err(error) => vec![Out::Note(format!("{}_GIVEN_UP {error:#}", self.track.note))],
        }
    }

    /// What came of the SUBSCRIBE: a subscription and its Joining FETCH, or
    /// a pause before it is asked for again.
    async fn settled(
        &mut self,
        session: &mut Session,
        asked: &Asked,
        sent: Result<Sent>,
    ) -> Vec<Out<T>> {
        let note = self.track.note;
        let subscription = match settle(session, asked, sent) {
            Ok(subscription) => subscription,
            Err(NotSubscribed::Refused(error)) => {
                let pause = self.again.after_refusal(error.retry_wait_ms());
                self.ask_in(pause);
                return vec![Out::Note(format!(
                    "{note}_REFUSED error_code={:#x}; asking again in {pause} ms",
                    error.error_code
                ))];
            }
            Err(NotSubscribed::Failed(error)) => {
                let pause = self.again.after_end();
                self.ask_in(pause);
                return vec![Out::Note(format!(
                    "{note}_FAILED {error:#}; asking again in {pause} ms"
                ))];
            }
        };
        self.again.subscribed();
        let mut out = vec![Out::Note(format!(
            "{note} track_alias={}",
            subscription.track_alias
        ))];
        self.asking_with
            .stops
            .limit_payload(subscription.track_alias, self.track.payload_most);
        let plan = history_to_fetch(
            self.shown.last(),
            subscription.largest.map(|largest| largest.group),
            self.track.catch_up,
        );
        let joining_request_id = subscription.request.value();
        self.subscription = Some(subscription);
        if plan.forget {
            self.shown.forget();
            out.push(Out::Note(format!(
                "{}_NUMBERED_ANEW",
                self.track.fetch_note
            )));
        }
        self.shown.refetching();
        // Nothing to fetch: no item yet, or none missed.
        let Some((joining_start, gap)) = plan.fetch else {
            out.extend(self.fetch_ended());
            return out;
        };
        // Told before the items after it come.
        if let Some(gap) = gap {
            out.push(Out::Gap(gap));
        }
        let range = FetchRange::AbsoluteJoining {
            joining_request_id,
            joining_start,
        };
        let tokens = self
            .asking_with
            .token
            .iter()
            .map(|token| Parameter::AuthorizationToken(token.clone()))
            .collect();
        let sent = match session.fetch(range, tokens) {
            Ok((fetch, message)) => send_request(
                &self.asking_with.connection,
                self.asking_with.open_wait_ms,
                &message,
                "fetch",
            )
            .await
            .map(|(request_send, answers)| (fetch, request_send, answers)),
            Err(error) => Err(error.into()),
        };
        match sent {
            Ok((fetch, request_send, answers)) => {
                self.kept.push(request_send);
                self.fetch = Some((fetch, answers));
            }
            Err(error) => {
                out.push(Out::Note(format!(
                    "{}_FETCH_FAILED {error:#}",
                    self.track.fetch_note
                )));
                out.extend(self.fetch_ended());
            }
        }
        out
    }

    /// What came on the stream of the subscription. The subscription that
    /// ends (the gatherer ends `lines` of a room nobody is in with
    /// `TRACK_ENDED`) is asked for again.
    fn on_answer(
        &mut self,
        session: &mut Session,
        event: Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>,
    ) -> Vec<Out<T>> {
        let Some(subscription) = self.subscription.as_ref() else {
            return Vec::new();
        };
        let request = subscription.request;
        let note = self.track.note;
        let how = match event {
            Ok(RequestStreamEvent::Message(message)) => {
                match session.on_request_message(request, message) {
                    Ok(RequestEvent::SubscriptionDone(done)) => {
                        format!("PUBLISH_DONE status_code={:#x}", done.status_code)
                    }
                    Ok(other) => {
                        return vec![Out::Note(format!("{note}_SAID {other:?}"))];
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
        if let Some(subscription) = self.subscription.take() {
            self.asking_with
                .stops
                .unlimit_payload(subscription.track_alias);
            subscription.cancel();
        }
        let pause = self.again.after_end();
        self.ask_in(pause);
        vec![Out::Note(format!(
            "{note}_ENDED {how}; asking again in {pause} ms"
        ))]
    }

    fn ask_in(&mut self, pause_ms: u64) {
        self.ask_at = Some(tokio::time::Instant::now() + Duration::from_millis(pause_ms));
    }

    /// Reads the stream of the FETCH on a task of its own; its items come
    /// back by [`Self::next`].
    pub fn read_fetch(&self, stream: u64, header: xmoq_moqt::fetch::FetchHeader) {
        let Some(mut body) = self.asking_with.stops.take_fetch_body(stream) else {
            return;
        };
        let to = self.fetched.0.clone();
        let most = self.track.fetch_most;
        tokio::spawn(async move {
            let mut decoder = FetchStreamDecoder::new(most, GroupOrder::Ascending);
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
    /// track without items yet is `INVALID_RANGE`, §10.12.3) ends the FETCH,
    /// and the items that waited are given. Returns the items to give.
    fn on_fetch_answer(
        &mut self,
        session: &mut Session,
        answer: Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>,
    ) -> Result<Vec<Out<T>>> {
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
            // Its stream brings the items, or brought them.
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

    /// The FETCH is over: the items that waited.
    fn fetch_ended(&mut self) -> Vec<Out<T>> {
        self.fetch = None;
        self.shown
            .fetch_ended()
            .into_iter()
            .map(|(_, item)| Out::Item(item))
            .collect()
    }

    /// An item of the FETCH, if it is given.
    fn fetched(&mut self, number: u64, payload: &[u8]) -> Option<Out<T>> {
        let item = (self.track.read)(number, payload)?;
        let (_, item) = self.shown.fetched(number, item)?;
        Some(Out::Item(item))
    }

    /// An Object of the track, the Group `number`: the item, if it is given.
    pub fn live(&mut self, number: u64, payload: &[u8]) -> Option<T> {
        let item = (self.track.read)(number, payload)?;
        let (_, item) = self.shown.live(number, item)?;
        Some(item)
    }
}
