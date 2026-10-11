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
use xmoq_moqt_io::{
    Connection, PeerRequestClose, SendStream, Trace, WriteError, open_fetch_stream, send_message,
};

use crate::published::Piece;
use crate::report;

/// FETCH requests of the peer that are answered at once; one more is
/// refused with `EXCESSIVE_LOAD`. As many as `@xmoq/moqt` answers.
/// Provisional.
///
/// WHY twice what the relay asks one publisher for (16): the relay stops
/// counting a FETCH when it cancels it or has its end, and this client when
/// it has taken the note of the task that wrote it, or read the reset. The
/// relay's next FETCH comes on a stream of its own and can be read first;
/// with 16 here it would be the 17th.
pub const MAX_ANSWERED_AT_ONCE: usize = 32;

/// How long this client's direction of the request stream of a FETCH stays
/// open after the fetch stream was written to its end, for the subscriber
/// to end its own direction first. Provisional; the relay and `@xmoq/moqt`
/// wait as long.
///
/// WHY wait at all: §5.2 has a subscriber's FETCH end with the FIN or the
/// reset of the fetch stream, with a `REQUEST_ERROR` or with its own
/// cancelling, and says nothing of a FIN on the request stream. The two
/// streams arrive apart, and `moqtransport` v0.14.0 fails a FETCH whose
/// request stream is finished before it has read the fetch stream to its
/// end. WHY NOT leave it open for good: a subscriber that never ends its
/// direction would keep the stream.
pub const PEER_END_WAIT: Duration = Duration::from_secs(3);

/// Notes of the tasks that are not taken yet. When it is full the tasks
/// wait.
const NOTE_QUEUE: usize = 16;

/// Messages for one request stream that its task has not sent yet. One more
/// is dropped: it is the `REQUEST_OK` of a `REQUEST_UPDATE` that changes
/// nothing, to a peer that takes none of them.
const SAY_QUEUE: usize = 16;

/// How this client answers a FETCH, from its flags: a publisher that is
/// slow in each of the places a relay waits for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    /// Dropped when the subscriber's direction of the request stream is
    /// over: the task of the stream waits for that before it finishes.
    peer_ends: oneshot::Sender<()>,
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

/// WHY: a client that leaves by an error does not go through
/// [`Answers::leave`], and a task that writes a fetch stream would go on
/// until the connection is closed.
impl Drop for Answers {
    fn drop(&mut self) {
        for answered in self.open.values() {
            answered.body.abort();
        }
    }
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
    /// The request streams of FETCH requests that are over, whose task
    /// waits for the subscriber's direction to end ([`PEER_END_WAIT`]):
    /// what is dropped when it has. An entry whose task gave up waiting is
    /// taken out when the next one is put in.
    lingering: HashMap<u64, (PeerRequestId, oneshot::Sender<()>)>,
    notes: mpsc::Receiver<Note>,
    queue: mpsc::Sender<Note>,
    pace: Pace,
}

impl Answers {
    pub fn new(pace: Pace) -> Self {
        let (queue, notes) = mpsc::channel(NOTE_QUEUE);
        Self {
            open: HashMap::new(),
            lingering: HashMap::new(),
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
    pub fn start<S, C>(
        &mut self,
        stream: u64,
        request: PeerRequestId,
        send: S,
        answer: Answer,
        connection: &Arc<C>,
        trace: &Trace,
    ) where
        S: SendStream + Send + 'static,
        C: Connection + Send + Sync + 'static,
    {
        let (says, said) = mpsc::channel(SAY_QUEUE);
        let (end, ended) = oneshot::channel();
        let (peer_ends, peer_ended) = oneshot::channel();
        let printed = Arc::new(Mutex::new(Printed::default()));
        tokio::spawn(speak(Speaker {
            stream,
            send,
            first: answer.messages,
            after: self.pace.answer_after,
            said,
            ended,
            peer_ended,
            end_wait: PEER_END_WAIT,
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
                peer_ends,
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
        let Some(answered) = self.open.get_mut(&stream) else {
            return match happened {
                // The subscriber's direction of a FETCH that is over has
                // ended: the task of the stream finishes this client's.
                Happened::PeerClosed(_) => {
                    self.lingering.remove(&stream);
                    None
                }
                // The subscriber stopped this client's direction while it
                // was kept open for it: the task has reset it, and the
                // subscriber's direction is stopped as well, as for a FETCH
                // that is cancelled while it is answered.
                Happened::RequestStopped(_) => {
                    let (request, _) = self.lingering.remove(&stream)?;
                    Some(Over {
                        request,
                        peer_gone: false,
                        reset: Some(stream_reset_code::CANCELLED),
                    })
                }
                _ => None,
            };
        };
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
        let peer_gone = answered.peer_finished
            || matches!(
                happened,
                Happened::PeerClosed(PeerRequestClose::Reset(_) | PeerRequestClose::Abandoned)
            );
        if !peer_gone && ended.request == RequestEnd::Finish {
            self.lingering
                .retain(|_, (_, peer_ends)| !peer_ends.is_closed());
            self.lingering
                .insert(stream, (answered.request, answered.peer_ends));
        }
        Some(Over {
            request: answered.request,
            reset: match ended.request {
                RequestEnd::Finish => None,
                RequestEnd::Reset(code) => Some(code),
            },
            peer_gone,
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
    /// Completes when the subscriber's direction of the stream is over, or
    /// nobody is left to say so.
    peer_ended: oneshot::Receiver<()>,
    /// How long a direction that is to be finished waits for that
    /// ([`PEER_END_WAIT`]).
    end_wait: Duration,
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
        peer_ended,
        end_wait,
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
    // Whether the subscriber sent `STOP_SENDING` for this direction.
    let mut stop_seen = false;
    // Whether the client can still hand over a message.
    let mut said_more = true;
    loop {
        tokio::select! {
            end = &mut ended => {
                match end {
                    // WHY what waits is sent first: §5.2 has "The publisher
                    // MUST send exactly one FETCH_OK or REQUEST_ERROR in
                    // response to a FETCH", and the fetch stream can be
                    // written to its end at the very moment the answer is
                    // due, or before this task ran once. A `REQUEST_OK`
                    // that was handed over and not taken yet goes with it.
                    //
                    // WHY NOT a limit on these writes: they wait only while
                    // the subscriber takes nothing on a stream that carries
                    // a few bytes, and fail when the connection goes, which
                    // the client's own timeout sees to.
                    Ok(RequestEnd::Finish) => {
                        while let Ok(message) = said.try_recv() {
                            waiting.push(message);
                        }
                        for message in waiting.drain(..) {
                            let _ = send_message(&mut send, &message, &trace, &label).await;
                        }
                        // The subscriber ends its direction first, if it
                        // does so in time ([`PEER_END_WAIT`]). WHY a
                        // `STOP_SENDING` meanwhile resets this direction,
                        // where the FETCH is over: §5.2 has "The Publisher
                        // can destroy fetch state as soon as it has
                        // received a STOP_SENDING. It MUST reset the bidi
                        // request stream", and the relay does so.
                        //
                        // WHY also a stop that was noted before the end was
                        // heard: the fetch stream can be written to its end
                        // at the moment the subscriber stops this
                        // direction.
                        if !stop_seen {
                            tokio::select! {
                                // WHY the stop first: one that is there
                                // already is not passed over for a wait
                                // that is over as well.
                                biased;
                                stopped = send.stopped(), if watched => match stopped {
                                    WriteError::Stopped(code) => {
                                        stop_seen = true;
                                        let stopped = Happened::RequestStopped(code);
                                        // The client is gone when this fails.
                                        let _ = notes.send(Note(stream, stopped)).await;
                                    }
                                    // The connection is gone, and the stream.
                                    WriteError::Lost(_) => return,
                                },
                                // Ended, nobody left to say so, or time.
                                _ = tokio::time::timeout(end_wait, peer_ended) => {}
                            }
                        }
                        if stop_seen {
                            send.reset(stream_reset_code::CANCELLED);
                        } else {
                            send.finish();
                        }
                    }
                    // A FETCH that is given up is owed no answer.
                    Ok(RequestEnd::Reset(code)) => send.reset(code),
                    Err(_) => send.reset(stream_reset_code::CANCELLED),
                }
                return;
            }
            () = &mut answer, if !answered => answered = true,
            message = said.recv(), if answered && waiting.is_empty() && said_more => {
                match message {
                    Some(message) => waiting.push(message),
                    // The client let go of the FETCH; `ended` says how.
                    // WHY the queue is not asked again, and nothing is
                    // waited for here: the client lets go of the queue and
                    // says how the stream ends in one step, and this arm
                    // can be taken before the other. A wait in it would
                    // keep the task from ever hearing the end.
                    None => said_more = false,
                }
            }
            stopped = send.stopped(), if watched => {
                watched = false;
                if let WriteError::Stopped(code) = stopped {
                    stop_seen = true;
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
struct Body<C> {
    connection: Arc<C>,
    header: FetchHeader,
    group_order: GroupOrder,
    pieces: Vec<Piece>,
    pace: Pace,
    printed: Arc<Mutex<Printed>>,
    trace: Trace,
}

impl<C: Connection> Body<C> {
    /// Opens the fetch stream, writes every piece and finishes it. A
    /// `STOP_SENDING` of the subscriber ends it also while it waits between
    /// two pieces.
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
                        // The code the request stream is reset with for it
                        // (`step`).
                        writer.reset(stream_reset_code::INTERNAL_ERROR);
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
            if !self.pace.write_every.is_zero() {
                tokio::select! {
                    () = tokio::time::sleep(self.pace.write_every) => {}
                    stopped = writer.stopped() => return Err(stopped),
                }
            }
        }
        writer.finish();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::fetch::{FetchEvent, FetchObject, FetchStreamDecoder, RangeEnd};
    use xmoq_moqt::message::{ControlMessage, Fetch, FetchRange};
    use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
    use xmoq_moqt::session::{PeerRequest, PeerRequestOpened, Role, Session, Transport};
    use xmoq_moqt_io::memory::{self, MemoryConnection};
    use xmoq_moqt_io::{ReadError, RecvStream};

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

    /// What the task of a request stream is told: how its direction ends,
    /// and when the subscriber's has.
    type Told = (oneshot::Receiver<RequestEnd>, oneshot::Receiver<()>);

    /// Answers that hold the FETCH requests on the streams `0..count`, with
    /// tasks that do nothing.
    fn answering(count: u64) -> (Answers, Vec<Told>) {
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        let mut answers = Answers::new(Pace::default());
        let mut ends = Vec::new();
        for stream in 0..count {
            let (says, _said) = mpsc::channel(1);
            let (end, ended) = oneshot::channel();
            let (peer_ends, peer_ended) = oneshot::channel();
            ends.push((ended, peer_ended));
            answers.open.insert(
                stream,
                Answered {
                    request: asked(&mut session, stream * 2 + 1),
                    says,
                    end,
                    body: tokio::spawn(std::future::pending()),
                    printed: Arc::new(Mutex::new(Printed::default())),
                    peer_finished: false,
                    peer_ends,
                },
            );
        }
        (answers, ends)
    }

    /// What a request stream was made to do, in order.
    #[derive(Clone, Default)]
    struct Done(Arc<Mutex<Vec<String>>>);

    struct Recorded(Done);

    /// A request stream the subscriber stopped: every write fails.
    struct Stopped(Done);

    impl SendStream for Stopped {
        fn write_all(&mut self, _: &[u8]) -> impl Future<Output = Result<(), WriteError>> {
            self.0.0.lock().unwrap().push("write fails".to_string());
            // CANCELLED.
            std::future::ready(Err(WriteError::Stopped(Some(0x1))))
        }

        fn finish(self) {
            self.0.0.lock().unwrap().push("finish".to_string());
        }

        fn finish_and_wait(self) -> impl Future<Output = Result<(), WriteError>> {
            std::future::ready(Err(WriteError::Stopped(Some(0x1))))
        }

        fn reset(self, code: u32) {
            self.0.0.lock().unwrap().push(format!("reset {code:#x}"));
        }

        fn stopped(&mut self) -> impl Future<Output = WriteError> {
            std::future::ready(WriteError::Stopped(Some(0x1)))
        }
    }

    impl SendStream for Recorded {
        fn write_all(&mut self, bytes: &[u8]) -> impl Future<Output = Result<(), WriteError>> {
            // The type of the message that begins there.
            let first = bytes.first().copied().unwrap_or(0);
            self.0.0.lock().unwrap().push(format!("write {first:#x}"));
            std::future::ready(Ok(()))
        }

        fn finish(self) {
            self.0.0.lock().unwrap().push("finish".to_string());
        }

        fn finish_and_wait(self) -> impl Future<Output = Result<(), WriteError>> {
            std::future::ready(Ok(()))
        }

        fn reset(self, code: u32) {
            self.0.0.lock().unwrap().push(format!("reset {code:#x}"));
        }

        async fn stopped(&mut self) -> WriteError {
            std::future::pending().await
        }
    }

    /// Runs [`speak`] with `FETCH_OK` to send after `answer_after`. After
    /// `end_after` the client hands over `said` messages and ends the stream.
    async fn spoken(
        answer_after: Duration,
        end_after: Duration,
        said: usize,
        end: RequestEnd,
    ) -> Vec<String> {
        // The subscriber's direction is over: nothing is waited for.
        let (_, peer_ended) = oneshot::channel();
        let wait = (peer_ended, Duration::ZERO);
        spoken_on(Recorded, answer_after, end_after, said, end, wait).await
    }

    /// The same on the request stream `stream_of` makes.
    async fn spoken_on<S: SendStream + Send + 'static>(
        stream_of: fn(Done) -> S,
        answer_after: Duration,
        end_after: Duration,
        said: usize,
        end: RequestEnd,
        (peer_ended, end_wait): (oneshot::Receiver<()>, Duration),
    ) -> Vec<String> {
        let done = Done::default();
        let (says, taken) = mpsc::channel(SAY_QUEUE);
        let (ends, ended) = oneshot::channel();
        let (notes, _notes) = mpsc::channel(NOTE_QUEUE);
        let fetch_ok = ControlMessage::FetchOk(xmoq_moqt::message::FetchOk {
            end_of_track: false,
            end_location: Location {
                group: 1,
                object: 0,
            },
            parameters: vec![],
            track_properties: vec![],
        });
        let task = tokio::spawn(speak(Speaker {
            stream: 0,
            send: stream_of(done.clone()),
            first: vec![fetch_ok],
            after: answer_after,
            said: taken,
            ended,
            peer_ended,
            end_wait,
            notes,
            trace: Trace::default(),
        }));
        tokio::time::sleep(end_after).await;
        for _ in 0..said {
            let request_ok = xmoq_moqt::message::RequestOk::default();
            says.try_send(ControlMessage::RequestOk(request_ok))
                .unwrap();
        }
        ends.send(end).unwrap();
        task.await.unwrap();
        done.0.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn the_answer_is_sent_before_the_request_stream_is_finished_whenever_the_end_comes() {
        let ms = Duration::from_millis;
        // FETCH_OK is 0x18, REQUEST_OK 0x7.
        let answered = vec!["write 0x18", "finish"];
        let with_an_update = vec!["write 0x18", "write 0x7", "finish"];
        // (when the answer is due; when the stream is ended; REQUEST_OK
        //  messages handed over just before; how many times; what the
        //  stream was made to do)
        let cases = [
            // The end comes at the moment the answer is due, and before the
            // task ran once: which of the two the task sees first is drawn,
            // so many times.
            (ms(2), ms(2), 0, 200, &answered),
            (ms(0), ms(0), 0, 200, &answered),
            (ms(2), ms(2), 1, 200, &with_an_update),
            // Before the answer is due, and after it.
            (ms(40), ms(2), 0, 3, &answered),
            (ms(2), ms(40), 0, 3, &answered),
            (ms(2), ms(40), 1, 3, &with_an_update),
        ];
        for (answer_after, end_after, said, runs, expected) in cases {
            for run in 0..runs {
                assert_eq!(
                    &spoken(answer_after, end_after, said, RequestEnd::Finish).await,
                    expected,
                    "{answer_after:?} {end_after:?} {said}, run {run}"
                );
            }
        }
        // A FETCH that is given up before its answer is owed none: CANCELLED
        // (0x1).
        assert_eq!(
            spoken(ms(40), ms(2), 0, RequestEnd::Reset(0x1)).await,
            ["reset 0x1"]
        );
    }

    #[tokio::test]
    async fn the_end_is_heard_when_the_client_lets_go_of_the_queue_in_the_same_step() {
        // The client says how the stream ends and lets go of the queue of
        // messages at once (`Answers::happened`): the task sees both
        // together, and which of two such things it takes first is drawn.
        // Many times, after the answer was sent.
        for run in 0..100 {
            let done = Done::default();
            let (says, said) = mpsc::channel(SAY_QUEUE);
            let (ends, ended) = oneshot::channel();
            let (_, peer_ended) = oneshot::channel();
            let (notes, _notes) = mpsc::channel(NOTE_QUEUE);
            let task = tokio::spawn(speak(Speaker {
                stream: 0,
                send: Recorded(done.clone()),
                first: vec![ControlMessage::RequestOk(
                    xmoq_moqt::message::RequestOk::default(),
                )],
                after: Duration::ZERO,
                said,
                ended,
                peer_ended,
                end_wait: Duration::ZERO,
                notes,
                trace: Trace::default(),
            }));
            // The answer is sent, and the task waits.
            tokio::time::sleep(Duration::from_millis(2)).await;
            ends.send(RequestEnd::Finish).unwrap();
            drop(says);
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap_or_else(|_| panic!("run {run}: the task never ended"))
                .unwrap();
            assert_eq!(
                done.0.lock().unwrap().clone(),
                ["write 0x7", "finish"],
                "run {run}"
            );
        }
    }

    #[tokio::test]
    async fn the_request_stream_is_finished_when_the_subscribers_direction_has_ended_or_in_time() {
        let ms = Duration::from_millis;
        let finished = vec!["write 0x18", "finish"];
        // (when the subscriber's direction ends, `None` for never; how long
        //  the task waits for that; how its own direction ends; what the
        //  stream was made to do; the least and the most time that takes)
        let cases = [
            // The subscriber ends first: finished right after.
            (
                Some(ms(100)),
                ms(5_000),
                RequestEnd::Finish,
                &finished,
                ms(100),
                ms(2_000),
            ),
            // It never does: finished when the time is over.
            (
                None,
                ms(300),
                RequestEnd::Finish,
                &finished,
                ms(300),
                ms(2_000),
            ),
            // A direction that is reset waits for nothing.
            (
                None,
                ms(5_000),
                RequestEnd::Reset(0x1),
                &vec!["reset 0x1"],
                ms(0),
                ms(2_000),
            ),
        ];
        for (peer_ends_after, end_wait, end, expected, least, most) in cases {
            let (peer_ends, peer_ended) = oneshot::channel();
            let subscriber = tokio::spawn(async move {
                match peer_ends_after {
                    Some(after) => tokio::time::sleep(after).await,
                    None => std::future::pending().await,
                }
                drop(peer_ends);
            });
            let started = tokio::time::Instant::now();
            let wait = (peer_ended, end_wait);
            // WHY the answer is due later for a direction that is reset: one
            // that is due at once is written or not as the task happens to
            // run, and this case is about what the reset waits for.
            let answer_after = match end {
                RequestEnd::Finish => ms(0),
                RequestEnd::Reset(_) => ms(1_000),
            };
            let log = spoken_on(Recorded, answer_after, ms(0), 0, end, wait).await;
            let took = started.elapsed();
            subscriber.abort();
            assert_eq!(&log, expected, "{peer_ends_after:?} {end_wait:?}");
            assert!(
                least <= took && took < most,
                "{peer_ends_after:?} {end_wait:?}: {took:?}"
            );
        }
    }

    /// A request stream whose subscriber sends `STOP_SENDING` with the code
    /// 0x77 after 100 ms.
    struct StoppedLater(Done);

    impl SendStream for StoppedLater {
        fn write_all(&mut self, bytes: &[u8]) -> impl Future<Output = Result<(), WriteError>> {
            let first = bytes.first().copied().unwrap_or(0);
            self.0.0.lock().unwrap().push(format!("write {first:#x}"));
            std::future::ready(Ok(()))
        }

        fn finish(self) {
            self.0.0.lock().unwrap().push("finish".to_string());
        }

        fn finish_and_wait(self) -> impl Future<Output = Result<(), WriteError>> {
            std::future::ready(Ok(()))
        }

        fn reset(self, code: u32) {
            self.0.0.lock().unwrap().push(format!("reset {code:#x}"));
        }

        async fn stopped(&mut self) -> WriteError {
            tokio::time::sleep(Duration::from_millis(100)).await;
            WriteError::Stopped(Some(0x77))
        }
    }

    #[tokio::test]
    async fn a_stop_sending_while_the_request_stream_waits_for_the_subscriber_resets_it() {
        let ms = Duration::from_millis;
        // The subscriber never ends its direction, and the task would wait
        // 5 seconds for it. The stop comes after 100 ms: this client's
        // direction is reset with CANCELLED (0x1), and not finished.
        let (_peer_ends, peer_ended) = oneshot::channel();
        let started = tokio::time::Instant::now();
        let wait = (peer_ended, ms(5_000));
        let log = spoken_on(StoppedLater, ms(0), ms(0), 0, RequestEnd::Finish, wait).await;
        assert_eq!(log, ["write 0x18", "reset 0x1"]);
        assert!(started.elapsed() < ms(2_000), "{:?}", started.elapsed());

        // The client is told, and stops the subscriber's direction as for a
        // FETCH that is cancelled while it is answered.
        let (mut answers, _told) = answering(1);
        let over = answers.happened(0, Happened::Written).unwrap();
        assert_eq!((over.peer_gone, over.reset), (false, None));
        let stopped = answers.happened(0, Happened::RequestStopped(Some(0x77)));
        assert_eq!(
            stopped,
            Some(Over {
                request: over.request,
                peer_gone: false,
                reset: Some(0x1),
            })
        );
        // Once: nothing waits for the stream any more.
        assert_eq!(answers.lingering.len(), 0);
        assert_eq!(
            answers.happened(0, Happened::RequestStopped(Some(0x77))),
            None
        );
        // And a stop for a stream that is none of them is nothing.
        assert_eq!(answers.happened(9, Happened::RequestStopped(None)), None);
    }

    #[tokio::test]
    async fn the_task_of_a_request_stream_is_told_when_the_subscribers_direction_has_ended() {
        let closed = |how| Happened::PeerClosed(how);
        // (what happens to the FETCH on stream 0, in order; whether the
        //  task is told that the subscriber's direction has ended, after
        //  each)
        let cases: Vec<(&str, Vec<(Happened, bool)>)> = vec![
            (
                "written first",
                vec![
                    (Happened::Written, false),
                    (closed(PeerRequestClose::Fin), true),
                ],
            ),
            (
                "written, and then reset by the subscriber",
                vec![
                    (Happened::Written, false),
                    (closed(PeerRequestClose::Reset(Some(0x1))), true),
                ],
            ),
            (
                "the subscriber finished first",
                vec![
                    (closed(PeerRequestClose::Fin), false),
                    (Happened::Written, true),
                ],
            ),
            (
                "cancelled",
                vec![(closed(PeerRequestClose::Reset(None)), true)],
            ),
            ("never", vec![(Happened::Written, false)]),
        ];
        for (name, steps) in cases {
            let (mut answers, mut told) = answering(1);
            let (_ended, mut peer_ended) = told.remove(0);
            for (happened, ended) in steps {
                answers.happened(0, happened);
                // Told by what it waits for being dropped.
                assert_eq!(
                    peer_ended
                        .try_recv()
                        .is_err_and(|error| error == oneshot::error::TryRecvError::Closed),
                    ended,
                    "{name} {happened:?}"
                );
            }
        }
        // What waits is let go of when the next FETCH is over, where the
        // task has given up waiting.
        let (mut answers, told) = answering(3);
        answers.happened(0, Happened::Written);
        answers.happened(1, Happened::Written);
        assert_eq!(answers.lingering.len(), 2);
        drop(told);
        answers.happened(2, Happened::Written);
        assert_eq!(answers.lingering.len(), 1);
    }

    /// A request stream whose subscriber sends `STOP_SENDING` with the code
    /// `CANCELLED` (0x1) when the test says so.
    struct Stoppable(Done, Arc<tokio::sync::Notify>);

    impl SendStream for Stoppable {
        fn write_all(&mut self, bytes: &[u8]) -> impl Future<Output = Result<(), WriteError>> {
            let first = bytes.first().copied().unwrap_or(0);
            self.0.0.lock().unwrap().push(format!("write {first:#x}"));
            std::future::ready(Ok(()))
        }

        fn finish(self) {
            self.0.0.lock().unwrap().push("fin".to_string());
        }

        fn finish_and_wait(self) -> impl Future<Output = Result<(), WriteError>> {
            std::future::ready(Ok(()))
        }

        fn reset(self, code: u32) {
            self.0.0.lock().unwrap().push(format!("reset:{code:#x}"));
        }

        async fn stopped(&mut self) -> WriteError {
            self.1.notified().await;
            WriteError::Stopped(Some(0x1))
        }
    }

    /// What this client's direction of the request stream was ended with,
    /// `None` while it is open.
    fn ended_with(done: &Done) -> Option<String> {
        let log = done.0.lock().unwrap();
        log.iter()
            .find(|entry| !entry.starts_with("write"))
            .cloned()
    }

    /// How this client's direction ended: within a second, or `wait>fin`
    /// for a FIN that came when the 3 seconds of [`PEER_END_WAIT`] were
    /// over and not before.
    async fn ended_soon_or_late(done: &Done, name: &str) -> String {
        let ms = Duration::from_millis;
        tokio::time::sleep(ms(1_000)).await;
        if let Some(end) = ended_with(done) {
            return end;
        }
        tokio::time::sleep(ms(1_500)).await;
        assert_eq!(ended_with(done), None, "{name}");
        tokio::time::sleep(ms(1_000)).await;
        assert_eq!(ended_with(done).as_deref(), Some("fin"), "{name}");
        "wait>fin".to_string()
    }

    /// `FETCH_OK` and a fetch stream that carries nothing.
    fn empty_answer() -> Answer {
        Answer {
            messages: vec![ControlMessage::FetchOk(xmoq_moqt::message::FetchOk {
                end_of_track: false,
                end_location: Location {
                    group: 1,
                    object: 0,
                },
                parameters: vec![],
                track_properties: vec![],
            })],
            header: FetchHeader { request_id: 1 },
            group_order: GroupOrder::Ascending,
            pieces: vec![],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_request_stream_of_a_fetch_that_was_served_ends_as_the_shared_table_says() {
        // `fixtures/peer-fetch/served-end.tsv`, which the relay and
        // `@xmoq/moqt` read as well.
        let table = include_str!("../../../fixtures/peer-fetch/served-end.tsv");
        let ms = Duration::from_millis;
        let closed = |how| Happened::PeerClosed(how);
        let mut rows = 0;
        for row in table.lines() {
            let columns: Vec<&str> = row.split('\t').collect();
            let [name, before, after, own, peer, update] = columns[..] else {
                panic!("a row of 6 columns: {row}");
            };
            let (publisher, _subscriber) = memory::pair();
            let publisher = Arc::new(publisher);
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            let mut answers = Answers::new(Pace::default());
            let (done, stop) = (Done::default(), Arc::new(tokio::sync::Notify::new()));
            let answer = empty_answer();
            let request_stream = Stoppable(done.clone(), Arc::clone(&stop));
            let request = asked(&mut session, 1);
            answers.start(
                0,
                request,
                request_stream,
                answer,
                &publisher,
                &Trace::default(),
            );
            match before {
                "-" => {}
                "fin" => assert_eq!(answers.happened(0, closed(PeerRequestClose::Fin)), None),
                other => panic!("{name}: before {other}"),
            }
            // The fetch stream is written to its end.
            let Note(stream, written) = answers.next_note().await;
            assert_eq!((stream, written), (0, Happened::Written), "{name}");
            let over = answers.happened(0, written).expect("the FETCH is over");
            // What the client does to the subscriber's direction: it stops
            // it with the code its own was reset with (`fetch_over` in
            // `main.rs`).
            let mut stops_peer = over.reset;

            let mut seen = None;
            let mut asked_again = false;
            for step in after.split(',') {
                match step {
                    "nothing" => {}
                    // 3 seconds: open for 2.5 of them, finished a second
                    // later.
                    "wait" => {
                        tokio::time::sleep(ms(2_500)).await;
                        assert_eq!(ended_with(&done), None, "{name}");
                        tokio::time::sleep(ms(1_000)).await;
                        assert_eq!(ended_with(&done).as_deref(), Some("fin"), "{name}");
                        seen = Some("wait>fin".to_string());
                    }
                    "fin" => {
                        answers.happened(0, closed(PeerRequestClose::Fin));
                    }
                    "reset" => {
                        answers.happened(0, closed(PeerRequestClose::Reset(Some(0x1))));
                    }
                    "stop-sending" => {
                        stop.notify_one();
                        // The task of the stream tells the client, where it
                        // still has the stream.
                        let told = tokio::time::timeout(ms(500), answers.next_note()).await;
                        if let Ok(Note(stream, stopped)) = told
                            && let Some(over) = answers.happened(stream, stopped)
                        {
                            stops_peer = over.reset;
                        }
                    }
                    // A REQUEST_UPDATE: the client has nothing to answer it
                    // with, and hands it to its session alone.
                    "update" => {
                        assert_eq!(answers.request_on(0), None, "{name}");
                        asked_again = true;
                    }
                    other => panic!("{name}: after {other}"),
                }
            }
            assert_eq!(asked_again, update == "no", "{name}");
            let seen = match seen {
                Some(seen) => seen,
                None => ended_soon_or_late(&done, name).await,
            };
            assert_eq!(seen, own, "{name}");
            let stopped = stops_peer.map_or("-".to_string(), |code| format!("stop:{code:#x}"));
            assert_eq!(stopped, peer, "{name}");
            // Ended once: nothing is put on the direction after its FIN or
            // its reset, whatever comes later.
            tokio::time::sleep(ms(4_000)).await;
            let ends = done.0.lock().unwrap().clone();
            let ends: Vec<&String> = ends
                .iter()
                .filter(|end| !end.starts_with("write"))
                .collect();
            assert_eq!(ends.len(), 1, "{name}: {ends:?}");
            rows += 1;
        }
        // A table that lost rows would pass.
        assert_eq!(rows, 10);
    }

    fn fetch_object(group_id: u64) -> FetchObject {
        FetchObject {
            group_id,
            subgroup_id: Some(0),
            object_id: 0,
            publisher_priority: 128,
            properties: vec![],
            payload: vec![0x61, 0x62],
        }
    }

    /// A body that writes `pieces` on `connection`, waiting `write_every`
    /// after each; what it counts; and the connection, which is closed
    /// when the last holder lets go of it.
    fn body(
        connection: MemoryConnection,
        pieces: Vec<Piece>,
        write_every: Duration,
    ) -> (
        Body<MemoryConnection>,
        Arc<Mutex<Printed>>,
        Arc<MemoryConnection>,
    ) {
        let counted = Arc::new(Mutex::new(Printed::default()));
        let connection = Arc::new(connection);
        let body = Body {
            connection: Arc::clone(&connection),
            header: FetchHeader { request_id: 3 },
            group_order: GroupOrder::Ascending,
            pieces,
            pace: Pace {
                write_every,
                ..Pace::default()
            },
            printed: Arc::clone(&counted),
            trace: Trace::default(),
        };
        (body, counted, connection)
    }

    #[tokio::test]
    async fn a_fetch_stream_carries_its_header_and_what_the_answer_has_in_order() {
        let (publisher, subscriber) = memory::pair();
        let unknown = Location {
            group: 1,
            object: 0,
        };
        let pieces = vec![
            Piece::EndOfRange(RangeEnd::Unknown, unknown),
            Piece::Object(fetch_object(2)),
            Piece::Object(fetch_object(3)),
        ];
        let (body, counted, _open) = body(publisher, pieces, Duration::ZERO);
        assert_eq!(body.write().await, Ok(()));
        // The two Objects, and not the indicator.
        assert_eq!(printed(&counted).objects, 2);

        let mut stream = subscriber.accept_uni().await.unwrap();
        let mut decoder = FetchStreamDecoder::new(1024, GroupOrder::Ascending);
        let mut events = Vec::new();
        let mut buffer = [0u8; 64];
        while let Some(read) = stream.read(&mut buffer).await.unwrap() {
            decoder.push(&buffer[..read]).unwrap();
            while let Some(event) = decoder.next_event().unwrap() {
                events.push(event);
            }
        }
        decoder.finish().unwrap();
        assert_eq!(
            events,
            [
                FetchEvent::Header(FetchHeader { request_id: 3 }),
                FetchEvent::EndOfRange {
                    kind: RangeEnd::Unknown,
                    location: unknown,
                },
                FetchEvent::Object(fetch_object(2)),
                FetchEvent::Object(fetch_object(3)),
            ]
        );
    }

    #[tokio::test]
    async fn a_fetch_stream_that_is_stopped_while_the_writer_waits_ends_there() {
        let (publisher, subscriber) = memory::pair();
        // One Object, and a wait after it that outlasts the test: without
        // an eye on the stream the wait would end and the stream finish.
        let (body, counted, _open) = body(
            publisher,
            vec![Piece::Object(fetch_object(2))],
            Duration::from_secs(600),
        );
        let writing = tokio::spawn(body.write());
        let stream = subscriber.accept_uni().await.unwrap();
        // CANCELLED (0x1).
        stream.stop(0x1);
        let ended = tokio::time::timeout(Duration::from_secs(5), writing).await;
        assert_eq!(
            ended.expect("in time").expect("no panic"),
            Err(WriteError::Stopped(Some(0x1)))
        );
        assert_eq!(printed(&counted).objects, 1);
    }

    #[tokio::test]
    async fn an_answer_that_cannot_be_written_still_ends_the_request_stream_once() {
        let ms = Duration::from_millis;
        // The subscriber stopped the client's direction. The answer is
        // tried once, whether the end comes with it or after it, and the
        // direction is reset with CANCELLED (0x1), as §5.2 has it for a
        // publisher that received STOP_SENDING, and not finished.
        for (answer_after, end_after) in [(ms(2), ms(40)), (ms(0), ms(0))] {
            assert_eq!(
                spoken_on(
                    Stopped,
                    answer_after,
                    end_after,
                    0,
                    RequestEnd::Finish,
                    (oneshot::channel().1, Duration::ZERO)
                )
                .await,
                ["write fails", "reset 0x1"],
                "{answer_after:?} {end_after:?}"
            );
        }
    }

    /// Reads a stream to where it ends: `None` for a FIN.
    async fn end_of(mut stream: impl RecvStream) -> Option<ReadError> {
        let mut buffer = [0u8; 64];
        let reading = async {
            loop {
                match stream.read(&mut buffer).await {
                    Ok(Some(_)) => {}
                    Ok(None) => return None,
                    Err(error) => return Some(error),
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(5), reading)
            .await
            .expect("in time")
    }

    #[tokio::test]
    async fn a_fetch_stream_whose_object_cannot_be_encoded_is_reset() {
        let (publisher, subscriber) = memory::pair();
        // Group 3 and then Group 2, in Ascending order: the encoder refuses
        // the second.
        let pieces = vec![
            Piece::Object(fetch_object(3)),
            Piece::Object(fetch_object(2)),
        ];
        let (body, counted, _open) = body(publisher, pieces, Duration::ZERO);
        assert!(matches!(body.write().await, Err(WriteError::Lost(_))));
        assert_eq!(printed(&counted).objects, 1);
        // INTERNAL_ERROR (0x0), the code the request stream gets for it.
        let stream = subscriber.accept_uni().await.unwrap();
        assert_eq!(end_of(stream).await, Some(ReadError::Reset(Some(0x0))));
    }

    #[tokio::test]
    async fn the_fetch_streams_end_with_the_answers_that_are_let_go_of() {
        let (publisher, subscriber) = memory::pair();
        let publisher = Arc::new(publisher);
        let mut session = Session::new(Role::Client, Transport::WebTransport);
        // A wait after each Object that outlasts the test.
        let mut answers = Answers::new(Pace {
            write_every: Duration::from_secs(600),
            ..Pace::default()
        });
        let answer = Answer {
            messages: vec![],
            header: FetchHeader { request_id: 1 },
            group_order: GroupOrder::Ascending,
            pieces: vec![
                Piece::Object(fetch_object(2)),
                Piece::Object(fetch_object(3)),
            ],
        };
        let request = asked(&mut session, 1);
        let request_stream = Recorded(Done::default());
        answers.start(
            0,
            request,
            request_stream,
            answer,
            &publisher,
            &Trace::default(),
        );
        let stream = subscriber.accept_uni().await.unwrap();
        // The client leaves by an error, without `leave`: the task that
        // writes is stopped, and its stream reset with CANCELLED (0x1).
        drop(answers);
        assert_eq!(end_of(stream).await, Some(ReadError::Reset(Some(0x1))));
    }

    #[tokio::test]
    async fn only_so_many_fetch_requests_are_answered_at_once() {
        // The expectations below are spelled out for a limit of 32.
        assert_eq!(MAX_ANSWERED_AT_ONCE, 32);
        // (FETCH requests that are answered; whether one more is taken)
        for (open, room) in [(0, true), (31, true), (32, false)] {
            assert_eq!(answering(open).0.has_room(), room, "{open}");
        }
        // One that is over makes room for the next.
        let (mut answers, _ends) = answering(32);
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
            // This client stopped the subscriber's direction, or the
            // connection is gone: nothing is left of the stream.
            (
                None,
                Happened::PeerClosed(PeerRequestClose::Abandoned),
                RequestEnd::Reset(0x1),
                true,
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
            assert_eq!(ends[0].0.try_recv(), Ok(request), "{before:?} {happened:?}");
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
