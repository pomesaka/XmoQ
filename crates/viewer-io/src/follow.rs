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
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::message::{ControlMessage, FetchRange};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{RequestEvent, RequestId, Session};
use xmoq_moqt::{RequestFault, stream_reset_code};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, RequestReader, RequestStreamEvent, SendStream, StreamEnd, Trace,
    send_message,
};
use xmoq_viewer::chat_track::{LinesGap, Shown, history_to_fetch};

pub use crate::kept::Asking;
use crate::kept::{self, KeptTrack, Said, Wanted, on};

/// The `RENDEZVOUS_TIMEOUT` (§10.2.6) of the SUBSCRIBE of a track that is
/// followed and whose publisher may come after the reader: the chat's
/// `lines`, a transcript's `speech`, and the catalog of a broadcast that has
/// not begun ([`crate::session::Requests::catalog`]). **Provisional.** WHY
/// 30 s: the value decided for a followed track on 2026-10-26
/// (`docs/architecture.md`, "The roles of an endpoint"), the most the own
/// relay holds a SUBSCRIBE (`MAX_RENDEZVOUS_MS` of `xmoq-relay-core`).
pub const RENDEZVOUS_MS: u64 = 30_000;

/// What the follower has to be told of, from the loop of the session.
pub enum Came<C: Connection = WtConnection> {
    /// An item of the FETCH, by its number.
    Fetched(u64, Vec<u8>),
    /// The FETCH's stream ended.
    FetchEnded {
        /// Read to its FIN; not where it was reset, lost, stopped, or broke
        /// the object model.
        whole: bool,
    },
    /// What came on the FETCH's request stream.
    FetchAnswer(Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>),
    /// What came of the subscription, or the time to ask for it.
    Kept(kept::Came<C>),
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
pub struct Follower<T, C: Connection = WtConnection> {
    track: Track<T>,
    kept: KeptTrack<C>,
    shown: Shown<T>,
    /// The Joining FETCH, while its answer may still come.
    fetch: Option<(RequestId, RequestReader<C::Recv>)>,
    /// The Joining FETCH of the subscription, while its items may still
    /// come, with the sending half of its request stream: open until then,
    /// for a malformed track to reset it (§2.4.2), and finished when the
    /// FETCH is over (§5.2), which gives its place among the streams the
    /// relay allows back.
    joining: Option<(RequestId, C::Send)>,
    /// The stream of the Joining FETCH's items, while it is read.
    fetch_stream: Option<(RequestId, u64)>,
    fetched: (mpsc::Sender<Came<C>>, mpsc::Receiver<Came<C>>),
}

/// Opens the stream of a request and sends its message, within `wait_ms`.
///
/// # Errors
///
/// A stream the relay does not allow in time, and one that breaks before
/// the message is written.
pub async fn send_request<C: Connection>(
    connection: &C,
    wait_ms: u64,
    message: &ControlMessage,
    label: &str,
) -> Result<(C::Send, RequestReader<C::Recv>)> {
    let wait = Duration::from_millis(wait_ms);
    let Ok(opened) = tokio::time::timeout(wait, connection.open_bi()).await else {
        bail!("the relay allowed no stream for {label} within {wait_ms} ms");
    };
    let (mut send, recv) = opened?;
    let trace = Trace::default();
    send_message(&mut send, message, &trace, label).await?;
    Ok((send, RequestReader::new(recv, &trace, label)))
}

impl<T, C: Connection> Follower<T, C> {
    /// A follower that asks for the track at once, from the loop. `had` is
    /// the newest item given on the sessions before, so that what was
    /// missed between them is fetched.
    #[must_use]
    pub fn new(track: Track<T>, asking_with: Asking<C>, had: Option<u64>) -> Self {
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
            joining: None,
            fetch_stream: None,
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
    pub async fn next(&mut self) -> Came<C> {
        let Self {
            fetched,
            fetch,
            kept,
            ..
        } = self;
        tokio::select! {
            came = fetched.1.recv() => came.unwrap_or(Came::FetchEnded { whole: false }),
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
        came: Came<C>,
    ) -> Result<Vec<Out<T>>, RequestFault> {
        Ok(match came {
            Came::Fetched(number, payload) => self.fetched(number, &payload).into_iter().collect(),
            Came::FetchEnded { whole: true } => {
                // Over in the session once it was answered too, so that
                // `cancel_malformed` does not name it later.
                if let Some((request, _)) = self.fetch_stream.take() {
                    session.on_fetch_stream_closed(request);
                }
                self.fetch_ended()
            }
            Came::FetchEnded { whole: false } => {
                self.give_up_fetch(session);
                self.fetch_ended()
            }
            Came::FetchAnswer(answer) => match self.on_fetch_answer(session, answer) {
                Ok(shown) => shown,
                Err(error) => {
                    let mut out = vec![Out::Note(format!(
                        "{}_FETCH_FAILED {error:#}",
                        self.track.fetch_note
                    ))];
                    self.give_up_fetch(session);
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

    /// The session ended the request `request` for a malformed track
    /// (`Session::cancel_malformed`): where it is the subscription or a
    /// FETCH of this track, its streams are reset and stopped with
    /// `MALFORMED_TRACK` (§2.4.2), and the subscription is asked for again
    /// ([`KeptTrack::malformed`]). The Joining FETCH that was still open is
    /// given up ([`Shown::fetch_given_up`]): the next one asks for its lines
    /// again. `None` where it is another's: a FETCH of this track that is
    /// over or was given up is not named, the session holds it no more.
    pub fn malformed(&mut self, request: RequestId) -> Option<Vec<Out<T>>> {
        if let Some(said) = self.kept.malformed(request) {
            return Some(self.note_of(said).into_iter().collect());
        }
        if self
            .joining
            .as_ref()
            .is_none_or(|(fetch, _)| *fetch != request)
        {
            return None;
        }
        if let Some((_, send)) = self.joining.take() {
            send.reset(stream_reset_code::MALFORMED_TRACK);
        }
        if let Some((_, answers)) = self.fetch.take() {
            answers.stop(stream_reset_code::MALFORMED_TRACK);
        }
        if let Some((_, stream)) = self.fetch_stream.take() {
            self.kept
                .asking_with()
                .stops
                .stop(stream, stream_reset_code::MALFORMED_TRACK);
        }
        // What its task read and did not hand over yet goes with it: the
        // next FETCH brings it again.
        self.fetched = mpsc::channel(32);
        self.shown.fetch_given_up();
        Some(vec![Out::Note(format!(
            "{}_FETCH_ENDED MALFORMED_TRACK",
            self.track.fetch_note
        ))])
    }

    /// What became of the subscription: on a new one, its Joining FETCH.
    async fn said(&mut self, session: &mut Session, said: Said) -> Vec<Out<T>> {
        match said {
            Said::Subscribed { .. } => self.subscribed(session).await,
            said => self.note_of(said).into_iter().collect(),
        }
    }

    /// The line for what became of the subscription; none for a new one,
    /// which [`Self::subscribed`] tells of.
    fn note_of(&self, said: Said) -> Option<Out<T>> {
        let note = self.track.note;
        Some(match said {
            Said::Subscribed { .. } => return None,
            Said::Ended { how, pause_ms } => {
                Out::Note(format!("{note}_ENDED {how}; asking again in {pause_ms} ms"))
            }
            Said::Refused {
                error_code,
                pause_ms,
            } => Out::Note(format!(
                "{note}_REFUSED error_code={error_code:#x}; asking again in {pause_ms} ms"
            )),
            Said::Failed { why, pause_ms } => Out::Note(format!(
                "{note}_FAILED {why}; asking again in {pause_ms} ms"
            )),
            Said::GivenUp(why) => Out::Note(format!("{note}_GIVEN_UP {why}")),
            Said::Other(other) => Out::Note(format!("{note}_SAID {other:?}")),
        })
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
                asking_with.connection.as_ref(),
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
                // One of a subscription before, whose items did not end.
                self.give_up_fetch(session);
                self.joining = Some((fetch, request_send));
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
    pub fn read_fetch(&mut self, stream: u64, header: xmoq_moqt::fetch::FetchHeader) {
        let Some(mut body) = self.kept.asking_with().stops.take_fetch_body(stream) else {
            return;
        };
        if let Some(joining) = self
            .joining
            .as_ref()
            .map(|(joining, _)| *joining)
            .filter(|joining| joining.value() == header.request_id)
        {
            self.fetch_stream = Some((joining, stream));
        }
        let to = self.fetched.0.clone();
        let most = self.track.fetch_most;
        tokio::spawn(async move {
            let mut decoder = FetchStreamDecoder::new(most, GroupOrder::Ascending);
            let mut whole = false;
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
                        FetchChunk::End(StreamEnd::Fin) => {
                            whole = true;
                            break;
                        }
                        FetchChunk::End(_) | FetchChunk::Stopped => break,
                    }
                }
            }
            let _ = to.send(Came::FetchEnded { whole }).await;
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

    /// Gives the Joining FETCH up before it is over, where one is open: in
    /// the session, and the directions of its request stream that are open
    /// with `CANCELLED` (§3.3.2; §5.2: "It MUST send `STOP_SENDING` for the
    /// bidi request stream"), and its fetch stream with it (§5.2 has it
    /// MAY). What its task read and did not hand over yet goes with it.
    ///
    /// WHY NOT a FIN, as for a FETCH that is over: the publisher may take a
    /// FIN of a FETCH it is still answering for nothing more than the end of
    /// what the subscriber says, and go on with the FETCH.
    fn give_up_fetch(&mut self, session: &mut Session) {
        let Some((fetch, send)) = self.joining.take() else {
            return;
        };
        session.cancel(fetch);
        send.reset(stream_reset_code::CANCELLED);
        if let Some((_, answers)) = self.fetch.take() {
            answers.stop(stream_reset_code::CANCELLED);
        }
        if let Some((_, stream)) = self.fetch_stream.take() {
            self.kept
                .asking_with()
                .stops
                .stop(stream, stream_reset_code::CANCELLED);
        }
        self.fetched = mpsc::channel(32);
    }

    /// The FETCH is over: the items that waited.
    fn fetch_ended(&mut self) -> Vec<Out<T>> {
        self.fetch = None;
        if let Some((_, send)) = self.joining.take() {
            send.finish();
        }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use xmoq_moqt::message::{
        FetchOk, ReasonPhrase, RequestError, SubscribeOk, request_error_code,
    };
    use xmoq_moqt::name::Location;
    use xmoq_moqt::session::{Role, Transport};
    use xmoq_moqt_io::memory::{self, MemoryConnection};
    use xmoq_moqt_io::{RecvStream, StreamStops};

    use super::*;

    /// How the Joining FETCH ends.
    #[derive(Debug, Clone, Copy)]
    enum Ends {
        /// Its items come, and their stream ends with a FIN.
        Read,
        /// The relay refuses it.
        Refused,
        /// It is answered, and its items' stream ends without a FIN.
        CutShort,
    }

    fn track() -> Track<u64> {
        Track {
            namespace: TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap(),
            name: "lines",
            rendezvous_ms: None,
            payload_most: 1024,
            fetch_most: 1024,
            catch_up: 10,
            note: "LINES",
            fetch_note: "HISTORY",
            read: |number, _| Some(number),
        }
    }

    async fn answer(
        relay: &MemoryConnection,
        message: ControlMessage,
    ) -> (impl SendStream, impl RecvStream) {
        let (mut send, recv) = relay.accept_bi().await.unwrap();
        send_message(&mut send, &message, &Trace::default(), "relay")
            .await
            .unwrap();
        (send, recv)
    }

    /// How `recv` ends: "fin", "reset <code>", or "open" after a second.
    async fn end_of(recv: &mut impl RecvStream) -> String {
        let reading = async {
            let mut buffer = [0; 256];
            loop {
                match recv.read(&mut buffer).await {
                    Ok(Some(_)) => {}
                    Ok(None) => return "fin".to_string(),
                    Err(error) => return format!("{error:?}"),
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(1), reading)
            .await
            .unwrap_or_else(|_| "open".to_string())
    }

    /// What `send` was stopped with, or "open" after a second.
    async fn stop_of(send: &mut impl SendStream) -> String {
        tokio::time::timeout(Duration::from_secs(1), send.stopped())
            .await
            .map_or_else(|_| "open".to_string(), |error| format!("{error:?}"))
    }

    /// A FETCH that is over has its request stream finished (§5.2); one
    /// given up before it is over has it reset and stopped with
    /// `CANCELLED` (§3.3.2, §5.2).
    #[tokio::test]
    async fn the_request_stream_of_a_joining_fetch_is_finished_when_the_fetch_is_over_and_cancelled_when_not()
     {
        // (how the FETCH ends; how the relay's reading half of its request
        //  stream ends, and its sending half where the FETCH was given up:
        //  one that is over has its answers let go of, and nothing more to
        //  be said there)
        let cases = [
            (Ends::Read, "fin", None),
            (Ends::Refused, "fin", None),
            (Ends::CutShort, "Reset(Some(1))", Some("Stopped(Some(1))")),
        ];
        for (ends, read, stopped) in cases {
            let (ours, relay) = memory::pair();
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            let asking_with = Asking {
                connection: Arc::new(ours),
                stops: StreamStops::default(),
                token: None,
                answer_wait_ms: 10_000,
                open_wait_ms: 10_000,
            };
            let mut follower = Follower::new(track(), asking_with, None);
            let ask_now = follower.next().await;
            assert!(follower.on(&mut session, ask_now).await.unwrap().is_empty());
            // A track with items: the Joining FETCH asks for the newest.
            let subscribe_ok = ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: 1,
                parameters: vec![Parameter::LargestObject(Location {
                    group: 5,
                    object: 0,
                })],
                track_properties: vec![],
            });
            let (settled, _subscription) =
                tokio::join!(follower.next(), answer(&relay, subscribe_ok));
            follower.on(&mut session, settled).await.unwrap();
            assert!(follower.fetch_request_id().is_some(), "{ends:?}");
            let fetch_answer = match ends {
                Ends::Read | Ends::CutShort => ControlMessage::FetchOk(FetchOk {
                    end_of_track: false,
                    end_location: Location {
                        group: 6,
                        object: 0,
                    },
                    parameters: vec![],
                    track_properties: vec![],
                }),
                Ends::Refused => ControlMessage::RequestError(RequestError {
                    error_code: request_error_code::INVALID_RANGE,
                    retry_interval: 0,
                    error_reason: ReasonPhrase::default(),
                    redirect: None,
                }),
            };
            let (mut fetch_send, mut fetch_recv) = answer(&relay, fetch_answer).await;
            let answered = follower.next().await;
            follower.on(&mut session, answered).await.unwrap();
            let whole = match ends {
                Ends::Read => Some(true),
                Ends::CutShort => Some(false),
                Ends::Refused => None,
            };
            if let Some(whole) = whole {
                follower
                    .on(&mut session, Came::FetchEnded { whole })
                    .await
                    .unwrap();
            }
            assert_eq!(end_of(&mut fetch_recv).await, read, "{ends:?}");
            if let Some(stopped) = stopped {
                assert_eq!(stop_of(&mut fetch_send).await, stopped, "{ends:?}");
            }
        }
    }
}
