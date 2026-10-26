//! Following a track of one item a Group, numbered by its Group ID without
//! a gap: the room's `lines` of the chat, and a transcript's `speech`. The
//! track is taken by SUBSCRIBE, with a `RENDEZVOUS_TIMEOUT` where its
//! publisher may come later, and its latest items by an Absolute Joining
//! FETCH; each item is given once, in the order of its number, and what was
//! missed while the subscription was away is fetched when it is made again
//! (`xmoq_viewer::chat_track` decides). The subscription is kept as
//! [`crate::kept`] keeps a track.
//!
//! WHY one follower for both: the gatherer and the transcriber publish their
//! tracks alike (one item a Group, from 0, and a FETCH answered from what
//! they keep), and a reader wants the same of either.

use std::time::Duration;

use anyhow::{Result, bail};
use tokio::sync::mpsc;
use xmoq_moqt::RequestFault;
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::message::{ControlMessage, FetchRange};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{RequestEvent, RequestId, Session};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, RequestReader, RequestStreamEvent, Trace, send_message,
};
use xmoq_viewer::chat_track::{LinesGap, Shown, history_to_fetch};

pub use crate::kept::Asking;
use crate::kept::{self, KeptTrack, Said, Wanted, on};

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

/// What the follower has to be told of, from the loop of the session.
pub enum Came {
    /// An item of the FETCH, by its number.
    Fetched(u64, Vec<u8>),
    /// The FETCH's stream ended, or no stream comes.
    FetchEnded,
    /// What came on the FETCH's request stream.
    FetchAnswer(Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>),
    /// What came of the subscription, or the time to ask for it.
    Kept(kept::Came),
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

/// One track followed on one session.
pub struct Follower<T> {
    track: Track<T>,
    kept: KeptTrack,
    shown: Shown<T>,
    /// The Joining FETCH, while its answer may still come.
    fetch: Option<(RequestId, RequestReader<Recv>)>,
    /// The request streams of each `FETCH`, which stay open while the
    /// session lasts.
    held: Vec<Send>,
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

impl<T> Follower<T> {
    /// A follower that asks for the track at once, from the loop. `had` is
    /// the newest item given on the sessions before, so that what was
    /// missed between them is fetched.
    #[must_use]
    pub fn new(track: Track<T>, asking_with: Asking, had: Option<u64>) -> Self {
        let wanted = Wanted {
            namespace: track.namespace.clone(),
            name: track.name.to_string(),
            rendezvous_ms: track.rendezvous_ms,
            payload_most: Some(track.payload_most),
            again_after_failure: true,
        };
        Self {
            track,
            kept: KeptTrack::new(wanted, asking_with),
            shown: Shown::fetching_after(had),
            fetch: None,
            held: Vec::new(),
            fetched: mpsc::channel(32),
        }
    }

    /// The number of the newest item given, if any.
    pub fn last_shown(&self) -> Option<u64> {
        self.shown.last()
    }

    /// The Track Alias of the track, while it is subscribed to.
    pub fn track_alias(&self) -> Option<u64> {
        self.kept.track_alias()
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
            kept,
            ..
        } = self;
        tokio::select! {
            came = fetched.1.recv() => came.unwrap_or(Came::FetchEnded),
            answer = on(fetch.as_mut().map(|(_, answers)| answers.next_event())) => Came::FetchAnswer(answer),
            came = kept.next() => Came::Kept(came),
        }
    }

    /// What came, and what the loop is to do with it.
    ///
    /// # Errors
    ///
    /// A message on the subscription's stream that breaks the protocol: the
    /// session is to be closed with its code ([`KeptTrack::on`]).
    pub async fn on(
        &mut self,
        session: &mut Session,
        came: Came,
    ) -> Result<Vec<Out<T>>, RequestFault> {
        Ok(match came {
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
            Came::Kept(came) => match self.kept.on(session, came)? {
                Some(said) => self.said(session, said).await,
                None => Vec::new(),
            },
        })
    }

    /// What became of the subscription: on a new one, its Joining FETCH.
    async fn said(&mut self, session: &mut Session, said: Said) -> Vec<Out<T>> {
        let note = self.track.note;
        match said {
            Said::Subscribed { .. } => self.subscribed(session).await,
            Said::Ended { how, pause_ms } => vec![Out::Note(format!(
                "{note}_ENDED {how}; asking again in {pause_ms} ms"
            ))],
            Said::Refused {
                error_code,
                pause_ms,
            } => vec![Out::Note(format!(
                "{note}_REFUSED error_code={error_code:#x}; asking again in {pause_ms} ms"
            ))],
            Said::Failed { why, pause_ms } => vec![Out::Note(format!(
                "{note}_FAILED {why}; asking again in {pause_ms} ms"
            ))],
            Said::GivenUp(why) => vec![Out::Note(format!("{note}_GIVEN_UP {why}"))],
            Said::Other(other) => vec![Out::Note(format!("{note}_SAID {other:?}"))],
        }
    }

    /// A subscription was made: the Joining FETCH of what was missed.
    async fn subscribed(&mut self, session: &mut Session) -> Vec<Out<T>> {
        let Some(subscription) = self.kept.subscription() else {
            return Vec::new();
        };
        let mut out = vec![Out::Note(format!(
            "{} track_alias={}",
            self.track.note, subscription.track_alias
        ))];
        let plan = history_to_fetch(
            self.shown.last(),
            subscription.largest.map(|largest| largest.group),
            self.track.catch_up,
        );
        let joining_request_id = subscription.request.value();
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
        let asking_with = self.kept.asking_with();
        let tokens = asking_with
            .token
            .iter()
            .map(|token| Parameter::AuthorizationToken(token.clone()))
            .collect();
        let sent = match session.fetch(range, tokens) {
            Ok((fetch, message)) => send_request(
                &asking_with.connection,
                asking_with.open_wait_ms,
                &message,
                "fetch",
            )
            .await
            .map(|(request_send, answers)| (fetch, request_send, answers)),
            Err(error) => Err(error.into()),
        };
        match sent {
            Ok((fetch, request_send, answers)) => {
                self.held.push(request_send);
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

    /// Reads the stream of the FETCH on a task of its own; its items come
    /// back by [`Self::next`].
    pub fn read_fetch(&self, stream: u64, header: xmoq_moqt::fetch::FetchHeader) {
        let Some(mut body) = self.kept.asking_with().stops.take_fetch_body(stream) else {
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
