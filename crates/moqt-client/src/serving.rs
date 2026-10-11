//! The FETCH requests of the peer this client answers: one task that speaks
//! on each request stream, one that writes each fetch stream, and the notes
//! they leave for the client, which alone speaks to the session.
//!
//! WHY tasks and not one more branch where the client waits: a subscriber
//! that does not read its fetch stream must not keep the publisher from
//! publishing, and the pace of answering is something the flags of this
//! client set on purpose (`--fetch-answer-delay-ms`,
//! `--fetch-stream-delay-ms`, `--fetch-write-every-ms`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use xmoq_moqt::fetch::{FetchHeader, FetchStreamEncoder};
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::params::GroupOrder;
use xmoq_moqt::session::PeerRequestId;
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    PeerRequestClose, SendStream, Trace, WriteError, open_fetch_stream, send_message,
};

use crate::published::Piece;
use crate::report;

/// FETCH requests of the peer that are answered at once; one more is
/// refused with `EXCESSIVE_LOAD`. As many as the relay asks one publisher
/// for, and as `@xmoq/moqt` answers. Provisional.
pub const MAX_ANSWERED_AT_ONCE: usize = 16;

/// Notes of the tasks that are not taken yet. When it is full the tasks
/// wait.
const NOTE_QUEUE: usize = 16;

/// Messages for one request stream that its task has not sent yet. One more
/// is dropped: it is the `REQUEST_OK` of a `REQUEST_UPDATE` that changes
/// nothing, to a peer that takes none of them.
const SAY_QUEUE: usize = 16;

/// How this client answers a FETCH, from its flags: a publisher that is
/// slow in each of the places a relay waits for it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pace {
    /// How long `FETCH_OK` waits.
    pub answer_after: Duration,
    /// How long the fetch stream waits after that before it is opened.
    pub stream_after: Duration,
    /// How long is waited after each Object and each indicator written.
    pub write_every: Duration,
}

/// What happened to a FETCH that was accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Happened {
    /// Everything was written, and the fetch stream finished.
    Written,
    /// The subscriber sent `STOP_SENDING` for the fetch stream.
    StreamStopped(Option<u32>),
    /// The fetch stream could not be opened or written.
    StreamFailed,
    /// The subscriber sent `STOP_SENDING` for this client's direction of the
    /// request stream.
    RequestStopped(Option<u32>),
    /// The subscriber's direction of the request stream is over.
    PeerClosed(PeerRequestClose),
    /// The client leaves while the FETCH is open.
    Left,
}

/// What becomes of this client's direction of the request stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestEnd {
    Finish,
    Reset(u32),
}

/// How an answered FETCH ended: what `FETCH_SERVED` prints, and what
/// becomes of its streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ended {
    pub how: &'static str,
    /// The code the subscriber gave, where it ended the FETCH with one.
    pub code: Option<u32>,
    pub request: RequestEnd,
}

/// The whole of what the client decides about a FETCH it accepted: `None`
/// where nothing follows. `@xmoq/moqt` decides the same
/// (`packages/moqt/test/core/session-peer-fetch.test.ts`).
///
/// Written to its end, both streams finish. Whatever else ends it resets
/// both, the fetch stream by the task that writes it being ended: §5.2 for
/// a subscriber that cancels on the request stream, by a reset or by
/// `STOP_SENDING`. WHY a `STOP_SENDING` for the fetch stream alone is read
/// as the FETCH given up, where the draft names only the request stream:
/// nothing more can be delivered, and the relay reads a subscriber of its
/// own that way. A subscriber that only finishes its direction of the
/// request stream has given nothing up (§3.3.2).
#[must_use]
pub fn step(happened: Happened) -> Option<Ended> {
    let cancelled = RequestEnd::Reset(stream_reset_code::CANCELLED);
    let (how, code, request) = match happened {
        Happened::Written => ("fin", None, RequestEnd::Finish),
        Happened::StreamStopped(code) => ("stream_stopped", code, cancelled),
        Happened::StreamFailed => (
            "stream_failed",
            None,
            RequestEnd::Reset(stream_reset_code::INTERNAL_ERROR),
        ),
        Happened::RequestStopped(code) => ("request_stopped", code, cancelled),
        Happened::PeerClosed(PeerRequestClose::Fin) => return None,
        Happened::PeerClosed(PeerRequestClose::Reset(code)) => ("request_reset", code, cancelled),
        // This endpoint stopped the stream, or the connection is gone.
        Happened::PeerClosed(PeerRequestClose::Abandoned) => ("abandoned", None, cancelled),
        Happened::Left => ("left", None, cancelled),
    };
    Some(Ended { how, code, request })
}

/// What a task has to tell the client about the FETCH on this request
/// stream.
#[derive(Debug)]
pub struct Note(pub u64, pub Happened);

/// What was printed for a FETCH.
#[derive(Debug, Default)]
struct Printed {
    /// Objects written to their end.
    objects: u64,
    /// Whether `FETCH_SERVED` was printed: no Object is counted after it.
    ended: bool,
}

/// A panic while a line was printed leaves the count as good as it gets.
fn printed(lines: &Mutex<Printed>) -> MutexGuard<'_, Printed> {
    lines
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One FETCH of the peer that was accepted and is not over.
struct Answered {
    request: PeerRequestId,
    /// Messages for the task of the request stream to send.
    says: mpsc::Sender<ControlMessage>,
    /// Ends the task of the request stream, and with it this client's
    /// direction; dropped, the direction is reset with `CANCELLED`.
    end: oneshot::Sender<RequestEnd>,
    body: JoinHandle<()>,
    /// WHY shared and locked: the task that writes the Objects runs beside
    /// the client, which prints `FETCH_SERVED` with their number; under one
    /// lock that line counts every Object written before it, and none is
    /// counted after it.
    printed: Arc<Mutex<Printed>>,
    /// Whether the subscriber has finished its direction of the request
    /// stream.
    peer_finished: bool,
}

/// A FETCH that is over, for the client to tell its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Over {
    pub request: PeerRequestId,
    /// Whether the subscriber's direction of the request stream is over as
    /// well: if not, the session keeps the request until it is.
    pub peer_gone: bool,
    /// The code this client's direction of the request stream was reset
    /// with, where it was not finished.
    pub reset: Option<u32>,
}

/// What a FETCH that is accepted is answered with.
pub struct Answer {
    /// `FETCH_OK`, and the `REQUEST_OK` of every `REQUEST_UPDATE` that
    /// waited for it.
    pub messages: Vec<ControlMessage>,
    pub header: FetchHeader,
    pub group_order: GroupOrder,
    pub pieces: Vec<Piece>,
}

/// The FETCH requests of the peer this client is answering, by their
/// request streams, and the queue of their notes.
pub struct Answers {
    open: HashMap<u64, Answered>,
    notes: mpsc::Receiver<Note>,
    queue: mpsc::Sender<Note>,
    pace: Pace,
}

impl Answers {
    pub fn new(pace: Pace) -> Self {
        let (queue, notes) = mpsc::channel(NOTE_QUEUE);
        Self {
            open: HashMap::new(),
            notes,
            queue,
            pace,
        }
    }

    /// Whether one more FETCH can be accepted.
    pub fn has_room(&self) -> bool {
        self.open.len() < MAX_ANSWERED_AT_ONCE
    }

    /// The request of the FETCH on `stream`, while it is answered.
    pub fn request_on(&self, stream: u64) -> Option<PeerRequestId> {
        self.open.get(&stream).map(|answered| answered.request)
    }

    /// Takes over the request stream of a FETCH the session accepted, and
    /// answers it.
    pub fn start<S: SendStream + Send + 'static>(
        &mut self,
        stream: u64,
        request: PeerRequestId,
        send: S,
        answer: Answer,
        connection: &Arc<WtConnection>,
        trace: &Trace,
    ) {
        let (says, said) = mpsc::channel(SAY_QUEUE);
        let (end, ended) = oneshot::channel();
        let printed = Arc::new(Mutex::new(Printed::default()));
        tokio::spawn(speak(Speaker {
            stream,
            send,
            first: answer.messages,
            after: self.pace.answer_after,
            said,
            ended,
            notes: self.queue.clone(),
            trace: trace.clone(),
        }));
        let body = Body {
            connection: Arc::clone(connection),
            header: answer.header,
            group_order: answer.group_order,
            pieces: answer.pieces,
            pace: self.pace,
            printed: Arc::clone(&printed),
            trace: trace.clone(),
        };
        let queue = self.queue.clone();
        let body = tokio::spawn(async move {
            let happened = match body.write().await {
                Ok(()) => Happened::Written,
                Err(WriteError::Stopped(code)) => Happened::StreamStopped(code),
                Err(WriteError::Lost(_)) => Happened::StreamFailed,
            };
            // The client is gone when this fails.
            let _ = queue.send(Note(stream, happened)).await;
        });
        self.open.insert(
            stream,
            Answered {
                request,
                says,
                end,
                body,
                printed,
                peer_finished: false,
            },
        );
    }

    /// The next note of a task; never, while no task has one.
    pub async fn next_note(&mut self) -> Note {
        match self.notes.recv().await {
            Some(note) => note,
            // Not reached: this holds a sender itself.
            None => std::future::pending().await,
        }
    }

    /// Has the task of the request stream send a message: the `REQUEST_OK`
    /// of a `REQUEST_UPDATE`.
    pub fn say(&self, stream: u64, message: ControlMessage) {
        if let Some(answered) = self.open.get(&stream) {
            // Full, or the task is done: see `SAY_QUEUE`.
            let _ = answered.says.try_send(message);
        }
    }

    /// Something happened to the FETCH on `stream`: ends its streams and
    /// prints `FETCH_SERVED` where [`step`] says so, once for every FETCH.
    /// Does nothing for a stream that has no FETCH that is answered.
    pub fn happened(&mut self, stream: u64, happened: Happened) -> Option<Over> {
        let answered = self.open.get_mut(&stream)?;
        let Some(ended) = step(happened) else {
            answered.peer_finished = true;
            return None;
        };
        let answered = self.open.remove(&stream)?;
        {
            let mut lines = printed(&answered.printed);
            lines.ended = true;
            report::fetch_served(
                answered.request.value(),
                ended.how,
                ended.code,
                lines.objects,
            );
        }
        // Ends the fetch stream where it is still written: the writer,
        // dropped, resets it with CANCELLED.
        answered.body.abort();
        // The task is gone when this fails, and its stream with it.
        let _ = answered.end.send(ended.request);
        Some(Over {
            request: answered.request,
            reset: match ended.request {
                RequestEnd::Finish => None,
                RequestEnd::Reset(code) => Some(code),
            },
            peer_gone: answered.peer_finished
                || matches!(
                    happened,
                    Happened::PeerClosed(PeerRequestClose::Reset(_) | PeerRequestClose::Abandoned)
                ),
        })
    }

    /// The client leaves: every FETCH that is open ends.
    pub fn leave(&mut self) -> Vec<Over> {
        let mut streams: Vec<u64> = self.open.keys().copied().collect();
        streams.sort_unstable();
        streams
            .into_iter()
            .filter_map(|stream| self.happened(stream, Happened::Left))
            .collect()
    }
}

/// What the task of a request stream needs.
struct Speaker<S> {
    stream: u64,
    send: S,
    /// Sent after `after`.
    first: Vec<ControlMessage>,
    after: Duration,
    said: mpsc::Receiver<ControlMessage>,
    ended: oneshot::Receiver<RequestEnd>,
    notes: mpsc::Sender<Note>,
    trace: Trace,
}

/// Speaks on the request stream of a FETCH that was accepted: `first` after
/// `after`, then what `said` brings, until `ended` says how the direction
/// ends. Notes a `STOP_SENDING` of the subscriber, once.
async fn speak<S: SendStream>(speaker: Speaker<S>) {
    let Speaker {
        stream,
        mut send,
        first,
        after,
        mut said,
        mut ended,
        notes,
        trace,
    } = speaker;
    let label = format!("bi#{stream}");
    let mut waiting = first;
    let answer = tokio::time::sleep(after);
    tokio::pin!(answer);
    let mut answered = false;
    // A `STOP_SENDING` is noted once; after it, and after the connection is
    // gone, the stream is not asked again.
    let mut watched = true;
    loop {
        tokio::select! {
            end = &mut ended => {
                match end {
                    Ok(RequestEnd::Finish) => send.finish(),
                    Ok(RequestEnd::Reset(code)) => send.reset(code),
                    Err(_) => send.reset(stream_reset_code::CANCELLED),
                }
                return;
            }
            () = &mut answer, if !answered => answered = true,
            message = said.recv(), if answered && waiting.is_empty() => match message {
                Some(message) => waiting.push(message),
                // The client let go of the FETCH; `ended` says how.
                None => std::future::pending().await,
            },
            stopped = send.stopped(), if watched => {
                watched = false;
                if let WriteError::Stopped(code) = stopped {
                    let _ = notes.send(Note(stream, Happened::RequestStopped(code))).await;
                }
            }
        }
        if answered {
            for message in waiting.drain(..) {
                // A failure is a `STOP_SENDING` or a lost connection, which
                // the branch above and the session's watcher report.
                let _ = send_message(&mut send, &message, &trace, &label).await;
            }
        }
    }
}

/// What the task of a fetch stream needs.
struct Body {
    connection: Arc<WtConnection>,
    header: FetchHeader,
    group_order: GroupOrder,
    pieces: Vec<Piece>,
    pace: Pace,
    printed: Arc<Mutex<Printed>>,
    trace: Trace,
}

impl Body {
    /// Opens the fetch stream, writes every piece and finishes it.
    async fn write(self) -> Result<(), WriteError> {
        tokio::time::sleep(self.pace.answer_after + self.pace.stream_after).await;
        let label = format!("fetch#{}", self.header.request_id);
        let mut writer = open_fetch_stream(
            &*self.connection,
            self.header,
            &self.trace,
            &label,
            stream_reset_code::CANCELLED,
        )
        .await?;
        let mut encoder = FetchStreamEncoder::new(self.header, self.group_order);
        // The header, which `open_fetch_stream` has written.
        encoder.take_bytes();
        for piece in &self.pieces {
            match piece {
                Piece::Object(object) => {
                    // An Object the encoder refuses is one this client made
                    // wrongly; the stream cannot go on without it.
                    if encoder.object(object).is_err() {
                        return Err(WriteError::Lost("an Object that cannot be encoded".into()));
                    }
                }
                Piece::EndOfRange(kind, location) => encoder.end_of_range(*kind, *location),
            }
            writer.write(&encoder.take_bytes()).await?;
            if matches!(piece, Piece::Object(_)) {
                let mut lines = printed(&self.printed);
                if !lines.ended {
                    lines.objects += 1;
                }
            }
            tokio::time::sleep(self.pace.write_every).await;
        }
        writer.finish();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::message::{ControlMessage, Fetch, FetchRange};
    use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
    use xmoq_moqt::session::{PeerRequest, PeerRequestOpened, Role, Session, Transport};

    use super::*;

    /// The FETCH with the Request ID `request_id` of a server, as the
    /// session of a client hands it on.
    fn asked(session: &mut Session, request_id: u64) -> PeerRequestId {
        let track = FullTrackName::new(
            TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap(),
            b"a".to_vec(),
        )
        .unwrap();
        let fetch = ControlMessage::Fetch(Fetch {
            request_id,
            range: FetchRange::Standalone {
                full_track_name: track,
                start: Location {
                    group: 0,
                    object: 0,
                },
                end: Location {
                    group: 1,
                    object: 0,
                },
            },
            parameters: vec![],
        });
        match session.on_peer_request_opened(fetch) {
            Ok(PeerRequestOpened::Request(request, PeerRequest::Fetch(_))) => request,
            other => panic!("{other:?}"),
        }
    }

    /// Answers that hold the FETCH requests on the streams `0..count`, with
    /// tasks that do nothing.
    fn answering(count: u64) -> (Answers, Vec<oneshot::Receiver<RequestEnd>>) {
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        let mut answers = Answers::new(Pace::default());
        let mut ends = Vec::new();
        for stream in 0..count {
            let (says, _said) = mpsc::channel(1);
            let (end, ended) = oneshot::channel();
            ends.push(ended);
            answers.open.insert(
                stream,
                Answered {
                    request: asked(&mut session, stream * 2 + 1),
                    says,
                    end,
                    body: tokio::spawn(std::future::pending()),
                    printed: Arc::new(Mutex::new(Printed::default())),
                    peer_finished: false,
                },
            );
        }
        (answers, ends)
    }

    #[tokio::test]
    async fn only_so_many_fetch_requests_are_answered_at_once() {
        // The expectations below are spelled out for a limit of 16.
        assert_eq!(MAX_ANSWERED_AT_ONCE, 16);
        // (FETCH requests that are answered; whether one more is taken)
        for (open, room) in [(0, true), (15, true), (16, false)] {
            assert_eq!(answering(open).0.has_room(), room, "{open}");
        }
        // One that is over makes room for the next.
        let (mut answers, _ends) = answering(16);
        assert!(answers.happened(3, Happened::Written).is_some());
        assert!(answers.has_room());
    }

    #[tokio::test]
    async fn a_fetch_that_is_over_ends_its_streams_once_and_says_what_is_left_of_it() {
        // CANCELLED (0x1).
        let cancelled = Some(0x1);
        // (what the subscriber did before; what ended the FETCH; how the
        //  request stream ends; whether the subscriber's direction is gone;
        //  the code it is stopped with)
        let cases = [
            (None, Happened::Written, RequestEnd::Finish, false, None),
            (
                Some(PeerRequestClose::Fin),
                Happened::Written,
                RequestEnd::Finish,
                true,
                None,
            ),
            (
                None,
                Happened::StreamStopped(Some(0x5)),
                RequestEnd::Reset(0x1),
                false,
                cancelled,
            ),
            (
                None,
                Happened::PeerClosed(PeerRequestClose::Reset(None)),
                RequestEnd::Reset(0x1),
                true,
                cancelled,
            ),
            (
                None,
                Happened::Left,
                RequestEnd::Reset(0x1),
                false,
                cancelled,
            ),
        ];
        for (before, happened, request, peer_gone, reset) in cases {
            let (mut answers, mut ends) = answering(1);
            let asked_for = answers.request_on(0).expect("answered");
            if let Some(how) = before {
                // Ends nothing by itself.
                assert_eq!(answers.happened(0, Happened::PeerClosed(how)), None);
            }
            assert_eq!(
                answers.happened(0, happened),
                Some(Over {
                    request: asked_for,
                    peer_gone,
                    reset,
                }),
                "{before:?} {happened:?}"
            );
            assert_eq!(ends[0].try_recv(), Ok(request), "{before:?} {happened:?}");
            // Over: whatever comes after finds no FETCH.
            assert_eq!(answers.happened(0, Happened::Left), None);
            assert_eq!(answers.request_on(0), None);
        }
    }

    #[test]
    fn what_happens_to_an_accepted_fetch_ends_its_streams_as_the_table_says() {
        let finish = Some(RequestEnd::Finish);
        // CANCELLED (0x1), and INTERNAL_ERROR (0x0).
        let cancel = Some(RequestEnd::Reset(0x1));
        let internal = Some(RequestEnd::Reset(0x0));
        // (what happened; the word of FETCH_SERVED; its code; what becomes
        //  of this client's direction of the request stream)
        let cases = [
            (Happened::Written, Some("fin"), None, finish),
            (
                Happened::StreamStopped(Some(0x1)),
                Some("stream_stopped"),
                Some(0x1),
                cancel,
            ),
            (
                Happened::StreamStopped(None),
                Some("stream_stopped"),
                None,
                cancel,
            ),
            (
                Happened::StreamFailed,
                Some("stream_failed"),
                None,
                internal,
            ),
            (
                Happened::RequestStopped(Some(0x5)),
                Some("request_stopped"),
                Some(0x5),
                cancel,
            ),
            (
                Happened::PeerClosed(PeerRequestClose::Reset(Some(0x1))),
                Some("request_reset"),
                Some(0x1),
                cancel,
            ),
            (
                Happened::PeerClosed(PeerRequestClose::Abandoned),
                Some("abandoned"),
                None,
                cancel,
            ),
            // A subscriber that only finishes its direction ends nothing.
            (
                Happened::PeerClosed(PeerRequestClose::Fin),
                None,
                None,
                None,
            ),
            (Happened::Left, Some("left"), None, cancel),
        ];
        for (happened, how, code, request) in cases {
            let ended = step(happened);
            assert_eq!(
                (
                    ended.map(|ended| ended.how),
                    ended.and_then(|ended| ended.code),
                    ended.map(|ended| ended.request),
                ),
                (how, code, request),
                "{happened:?}"
            );
        }
    }
}
