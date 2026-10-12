//! The FETCH requests this client sends: one task that reads each request
//! stream, one that reads and prints each fetch stream, and the notes they
//! leave for the client, which alone speaks to the session.
//!
//! WHY tasks and not one more branch where the client waits: a client can
//! have several FETCH requests open, each with two streams that go on at
//! their own pace, and the pace of reading a fetch stream is something the
//! flags of this client set on purpose (`--fetch-read-after-ms`,
//! `--fetch-read-every-ms`).

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use xmoq_moqt::fetch::{FetchEvent, FetchHeader, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::params::GroupOrder;
use xmoq_moqt::session::RequestId;
use xmoq_moqt_io::{FetchBody, FetchChunk, RequestClose, RequestStreamEvent, StreamEnd};

use crate::io::RequestStream;
use crate::report;

/// The largest Object of a fetch stream this client takes, in bytes, as for
/// a subgroup stream (`StreamConfig::subgroup_buffer_limit`). Provisional.
const FETCH_OBJECT_LIMIT: usize = 1024 * 1024;

/// Notes of the tasks that are not taken yet. When it is full the tasks
/// wait, and with them the streams they read.
const NOTE_QUEUE: usize = 16;

/// How this client reads a fetch stream, from its flags.
#[derive(Debug, Clone, Copy)]
pub struct Reading {
    /// How long nothing is taken from the stream after it opened.
    pub after: Duration,
    /// How long is waited after each piece of the stream that was taken: a
    /// read of the stream, 16 KiB at most, and not an Object.
    pub every: Duration,
    /// Give the FETCH up after this many Objects.
    pub cancel_after_objects: Option<u64>,
    /// Give the FETCH up this long after it was sent, whatever came of it.
    pub cancel_after: Option<Duration>,
    /// `--fetch-cancel-request-only`: a FETCH that is given up is cancelled
    /// on its request stream alone. Its fetch stream is not stopped, and
    /// not read any further.
    ///
    /// WHY a client that does so: §5.2 has "If the data stream is already
    /// open, the subscriber wishing to cancel the FETCH MAY send
    /// `STOP_SENDING` for the data stream as well as the bidi request
    /// stream. It MUST send `STOP_SENDING` for the bidi request stream", and
    /// of the publisher then, "It MUST reset the bidi request stream and
    /// unidirectional data stream associated with the FETCH". A subscriber
    /// that stops both streams itself never lets a publisher show that it
    /// resets the second.
    pub cancel_request_only: bool,
}

/// How the reading of a fetch stream ended.
#[derive(Debug)]
pub enum BodyEnd {
    Fin,
    Reset(Option<u32>),
    Lost,
    /// This client stopped the stream.
    Stopped,
    /// As many Objects as `--fetch-cancel-after-objects` came.
    GaveUp,
    /// What the stream carried could not be decoded.
    Broken(xmoq_moqt::Error),
}

/// What a task has to tell the client about the FETCH with this index.
#[derive(Debug)]
pub enum Note {
    Message(usize, ControlMessage),
    /// The peer ended its direction of the request stream, or stopped this
    /// client's.
    RequestClosed(usize, RequestClose),
    /// The request stream of a FETCH could not be read: the session is at
    /// fault, whichever FETCH it was.
    RequestBroken(Error),
    /// The reading of the fetch stream is over.
    Body(usize, BodyEnd),
    /// The time of `--fetch-cancel-after-ms` is over for this FETCH.
    CancelDue(usize),
    /// The time the answer to this FETCH gets is over
    /// (`xmoq_moqt::params::answer_wait_ms`).
    AnswerDue(usize),
}

/// How the fetch stream of a FETCH ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnded {
    Fin,
    /// Reset by the peer, with this code.
    Reset(Option<u32>),
}

/// What the client knows of one FETCH it sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    /// Whether `FETCH_OK` came.
    pub answered: bool,
    /// Whether the fetch stream has ended, and how.
    pub stream: Option<StreamEnded>,
    /// Whether the FETCH is over, for whatever reason.
    pub over: bool,
    /// Whether it is over because this client gave it up and reset its
    /// direction of the request stream.
    pub gave_up: bool,
}

impl Progress {
    /// Whether a message that comes on the request stream now is the
    /// session's to judge.
    ///
    /// Not after this client gave the FETCH up: the peer sent what crosses
    /// the reset before it saw it, and broke no rule. After a FETCH that
    /// came to its end, or was refused, the peer has said its last, and a
    /// message that follows is one the session finds out.
    #[must_use]
    pub fn hears_the_peer(self) -> bool {
        !self.gave_up
    }
}

/// What happened to a FETCH, as its tasks and the session report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Happened {
    /// `FETCH_OK` came.
    Answered,
    /// `REQUEST_ERROR` came.
    Refused,
    /// `FETCH_OK` came with a Mandatory Track Property (§2.5.1); the session
    /// has ended the request.
    UnsupportedTrackProperty,
    /// The peer ended its direction of the request stream, or stopped this
    /// client's, which changes nothing by itself.
    RequestClosed(RequestClose),
    /// The fetch stream ended, by the peer.
    StreamEnded(StreamEnded),
    /// The fetch stream was stopped by this client's readers.
    StreamStopped,
    /// As many Objects as `--fetch-cancel-after-objects` came.
    GaveUp,
    /// What the stream carried ends the stream and not the session
    /// (`xmoq_moqt::Error::scope`).
    StreamRefused,
    /// The client leaves while the FETCH is open.
    Left,
    /// The time its answer gets is over. Nothing for a FETCH that was
    /// answered.
    NotAnsweredInTime,
}

/// What the session has to hear of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tell {
    /// `Session::cancel`.
    Cancel,
    /// `Session::on_fetch_stream_closed`.
    StreamClosed,
}

/// How a FETCH ended: what `FETCH_ENDED` prints, and what becomes of its
/// streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ended {
    pub how: &'static str,
    /// The code of the reset that ended it, where one did.
    pub code: Option<u32>,
    /// Whether this client's direction of the request stream is reset
    /// (§3.3.2) and not finished. The fetch stream is stopped in either
    /// case, where it is still read.
    pub cancel: bool,
}

/// What follows from something that happened to a FETCH.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub progress: Progress,
    pub tell: Option<Tell>,
    /// Set once for every FETCH: `FETCH_ENDED` is printed for it.
    pub ended: Option<Ended>,
}

/// The whole of what the client decides about a FETCH of its own, apart from
/// errors that end the session. Both implementations decide the same
/// (implementation notes: rows on how long the sender of a FETCH keeps the
/// request, and on the publisher finishing or resetting its direction of
/// the request stream).
///
/// The FETCH is over when it was refused, when it was given up, or when
/// both its `FETCH_OK` came and its fetch stream ended, by a FIN or by a
/// reset: the answer "can come at any time relative to object delivery"
/// (§10.12.3), also after a reset, and a `REQUEST_ERROR` that follows a
/// reset says why.
///
/// The request stream: a FIN of the peer after the answer ends nothing;
/// before it, no answer can come any more, the FETCH is over, and this
/// client finishes its own direction. A reset of the peer ends the FETCH
/// whenever it comes, and this client resets its own direction. A
/// `STOP_SENDING` of the peer for this client's direction ends nothing by
/// itself: the FETCH ends when the peer's own direction does.
#[must_use]
pub fn step(progress: Progress, happened: Happened) -> Step {
    let nothing = Step {
        progress,
        tell: None,
        ended: None,
    };
    if progress.over {
        return nothing;
    }
    let ends = |how, code, cancel, tell| Step {
        progress: Progress {
            over: true,
            gave_up: cancel,
            ..progress
        },
        tell,
        ended: Some(Ended { how, code, cancel }),
    };
    // Answered and its stream ended: over, and how the stream ended is how
    // the FETCH did. This client finishes its direction of the request
    // stream.
    let served = |progress: Progress, tell| {
        let ended = match (progress.answered, progress.stream) {
            (true, Some(StreamEnded::Fin)) => Some(("fin", None)),
            (true, Some(StreamEnded::Reset(code))) => Some(("stream_reset", code)),
            (false, _) | (true, None) => None,
        };
        Step {
            progress: Progress {
                over: ended.is_some(),
                ..progress
            },
            tell,
            ended: ended.map(|(how, code)| Ended {
                how,
                code,
                cancel: false,
            }),
        }
    };
    match happened {
        Happened::Answered => served(
            Progress {
                answered: true,
                ..progress
            },
            None,
        ),
        Happened::StreamEnded(how) => served(
            Progress {
                stream: Some(how),
                ..progress
            },
            Some(Tell::StreamClosed),
        ),
        // The session has ended the request with the answer.
        Happened::Refused => ends("refused", None, false, None),
        Happened::UnsupportedTrackProperty => ends("unsupported_track_property", None, true, None),
        // The time was for the answer, not for the Objects.
        Happened::RequestClosed(RequestClose::Fin) | Happened::NotAnsweredInTime
            if progress.answered =>
        {
            nothing
        }
        Happened::RequestClosed(RequestClose::Fin) => ends(
            "request_finished_unanswered",
            None,
            false,
            Some(Tell::Cancel),
        ),
        Happened::RequestClosed(RequestClose::Reset(code)) => {
            ends("request_reset", code, true, Some(Tell::Cancel))
        }
        Happened::RequestClosed(RequestClose::Stopped(_)) => nothing,
        Happened::StreamStopped => ends("stream_stopped", None, true, Some(Tell::Cancel)),
        Happened::GaveUp => ends("cancelled", None, true, Some(Tell::Cancel)),
        Happened::StreamRefused => ends("stream_refused", None, true, Some(Tell::Cancel)),
        Happened::Left => ends("left", None, true, Some(Tell::Cancel)),
        Happened::NotAnsweredInTime => ends("no_answer_in_time", None, true, Some(Tell::Cancel)),
    }
}

/// One FETCH of this client.
pub struct OwnFetch {
    pub id: RequestId,
    group_order: GroupOrder,
    pub progress: Progress,
    /// The lines printed for the FETCH. WHY shared and locked: the task that
    /// prints its Objects runs beside the client, which prints `FETCH_ENDED`
    /// with their number; under one lock that line counts every Object line
    /// before it, and none comes after it.
    printed: Arc<Mutex<Printed>>,
    /// Ends the task of the request stream: sent, the request is cancelled
    /// (§3.3.2); dropped, this client's direction is finished.
    request: Option<oneshot::Sender<()>>,
    body: Option<JoinHandle<()>>,
    /// Tells the task of the fetch stream to read no more and to keep the
    /// stream as it is (`Reading::cancel_request_only`).
    hold: Arc<tokio::sync::Notify>,
    /// How long its answer gets, in milliseconds.
    pub answer_wait_ms: u64,
    /// The task that says when that time is over; stopped when the answer
    /// came, and when the FETCH is over.
    answer_timer: JoinHandle<()>,
}

/// What was printed for a FETCH.
#[derive(Debug, Default)]
struct Printed {
    /// `FETCH_OBJECT` lines.
    objects: u64,
    /// Whether `FETCH_ENDED` was printed: no line of the FETCH follows it.
    ended: bool,
}

/// A panic while a line was printed leaves the count as good as it gets.
fn printed(lines: &Mutex<Printed>) -> MutexGuard<'_, Printed> {
    lines
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl OwnFetch {
    /// Prints `FETCH_ENDED` for the FETCH, once, with the number of Object
    /// lines printed before it.
    pub fn print_ended(&self, ended: Ended) {
        let mut lines = printed(&self.printed);
        if !lines.ended {
            lines.ended = true;
            report::fetch_ended(self.id.value(), ended.how, ended.code, lines.objects);
        }
    }
}

/// The FETCH requests of this client and the queue of their notes.
pub struct Fetches {
    pub sent: Vec<OwnFetch>,
    /// A FETCH was given up because its answer did not come in time: the
    /// mode that sent it fails.
    pub unanswered: bool,
    notes: mpsc::Receiver<Note>,
    queue: mpsc::Sender<Note>,
    reading: Reading,
}

impl Fetches {
    pub fn new(reading: Reading) -> Self {
        let (queue, notes) = mpsc::channel(NOTE_QUEUE);
        Self {
            sent: Vec::new(),
            unanswered: false,
            notes,
            queue,
            reading,
        }
    }

    /// Takes over the request stream of a FETCH that was sent on it.
    /// `answer_wait` is how long its answer gets; then the FETCH is given
    /// up ([`Happened::NotAnsweredInTime`]).
    pub fn add(
        &mut self,
        id: RequestId,
        group_order: GroupOrder,
        stream: RequestStream,
        answer_wait: Duration,
    ) {
        let index = self.sent.len();
        let (request, cancelled) = oneshot::channel();
        tokio::spawn(watch_request(index, stream, cancelled, self.queue.clone()));
        let notes = self.queue.clone();
        let answer_timer = tokio::spawn(async move {
            tokio::time::sleep(answer_wait).await;
            // The client is gone when this fails.
            let _ = notes.send(Note::AnswerDue(index)).await;
        });
        if let Some(after) = self.reading.cancel_after {
            let notes = self.queue.clone();
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                // The client is gone when this fails.
                let _ = notes.send(Note::CancelDue(index)).await;
            });
        }
        self.sent.push(OwnFetch {
            id,
            group_order,
            progress: Progress::default(),
            printed: Arc::new(Mutex::new(Printed::default())),
            request: Some(request),
            body: None,
            hold: Arc::new(tokio::sync::Notify::new()),
            answer_wait_ms: u64::try_from(answer_wait.as_millis()).unwrap_or(u64::MAX),
            answer_timer,
        });
    }

    /// The answer of a FETCH came, or the FETCH is over: nothing waits for
    /// its answer any more.
    pub fn stop_answer_timer(&mut self, index: usize) {
        if let Some(fetch) = self.sent.get(index) {
            fetch.answer_timer.abort();
        }
    }

    /// The next note; never, when no FETCH is open.
    pub async fn next_note(&mut self) -> Note {
        if self.all_over() {
            return std::future::pending().await;
        }
        match self.notes.recv().await {
            Some(note) => note,
            // Not reached: this value holds a sender.
            None => std::future::pending().await,
        }
    }

    pub fn all_over(&self) -> bool {
        self.sent.iter().all(|fetch| fetch.progress.over)
    }

    /// The index of the open FETCH with this handle.
    pub fn index_of(&self, id: RequestId) -> Option<usize> {
        self.sent
            .iter()
            .position(|fetch| fetch.id == id && !fetch.progress.over)
    }

    /// Starts reading the fetch stream of a FETCH.
    pub fn read(&mut self, index: usize, header: FetchHeader, body: FetchBody) {
        let Some(fetch) = self.sent.get_mut(index) else {
            return;
        };
        let task = BodyTask {
            index,
            header,
            group_order: fetch.group_order,
            reading: self.reading,
            printed: Arc::clone(&fetch.printed),
            notes: self.queue.clone(),
        };
        let hold = Arc::clone(&fetch.hold);
        fetch.body = Some(tokio::spawn(async move {
            let mut body = body;
            tokio::select! {
                () = task.read(&mut body) => {}
                // Told to read no more: the stream is kept, unread and not
                // stopped, until this task ends with the client.
                () = hold.notified() => std::future::pending().await,
            }
        }));
    }

    /// Lets go of the streams of a FETCH that is over: the request stream
    /// is reset (§3.3.2) or finished, and a fetch stream that is still
    /// read is stopped. With `--fetch-cancel-request-only` the fetch stream
    /// of a FETCH this client gave up itself (`gave_up`) is kept instead,
    /// and read no further.
    ///
    /// WHY not of every FETCH that is cancelled: a FETCH whose request
    /// stream the peer reset, or whose fetch stream ended in a way the
    /// client does not take, is cancelled as well, and the flag is for the
    /// cancellation the client was told to make.
    pub fn end(&mut self, index: usize, cancel: bool, gave_up: bool) {
        let Some(fetch) = self.sent.get_mut(index) else {
            return;
        };
        match fetch.request.take() {
            // An error means the task is gone, and the stream with it.
            Some(request) if cancel => {
                let _ = request.send(());
            }
            // Dropped: the task finishes this client's direction.
            Some(_) | None => {}
        }
        if gave_up && self.reading.cancel_request_only {
            // Only a fetch stream that is open by now is kept. One that
            // opens after this belongs to a FETCH that is over: it is
            // dropped where it is handed over, which stops it, and once
            // every FETCH is over nobody takes streams from the session any
            // more, so that it stays there untaken until the client leaves.
            fetch.hold.notify_one();
            return;
        }
        // The body goes with its task, which stops the stream (`FetchBody`).
        if let Some(body) = fetch.body.take() {
            body.abort();
        }
    }
}

impl Drop for Fetches {
    /// WHY the line is printed here as well: the client can be cut off
    /// where it waits, by its own time limit or by an error of the session,
    /// and a driver that waits for `FETCH_ENDED` gets it for every FETCH
    /// that was sent all the same. The session is not told: it is on its
    /// way out.
    fn drop(&mut self) {
        for fetch in &mut self.sent {
            let left = step(fetch.progress, Happened::Left);
            if let Some(ended) = left.ended {
                fetch.print_ended(ended);
                // What `step` decided, as far as it can still be done: the
                // task resets the stream if it runs once more.
                if let Some(request) = fetch.request.take() {
                    let _ = request.send(());
                }
            }
            if let Some(body) = fetch.body.take() {
                body.abort();
            }
        }
    }
}

/// Reads the request stream of a FETCH until the peer ends it, and holds it
/// until the client says what becomes of it.
async fn watch_request(
    index: usize,
    mut stream: RequestStream,
    mut ending: oneshot::Receiver<()>,
    notes: mpsc::Sender<Note>,
) {
    let cancelled = loop {
        let event = tokio::select! {
            ending = &mut ending => break ending.is_ok(),
            event = stream.next_event() => event,
        };
        let (note, closed) = match event {
            Ok(RequestStreamEvent::Message(message)) => (Note::Message(index, message), false),
            Ok(RequestStreamEvent::Closed(how)) => (Note::RequestClosed(index, how), true),
            Err(error) => (Note::RequestBroken(error), true),
        };
        if notes.send(note).await.is_err() {
            break false;
        }
        if closed {
            // Nothing more is read. The client decides whether the FETCH
            // goes on (§10.12.3: the answer "can come at any time relative
            // to object delivery", and a FIN after it ends nothing).
            break ending.await.is_ok();
        }
    };
    if cancelled {
        stream.cancel();
    } else {
        stream.finish();
    }
}

/// What the task that reads a fetch stream needs.
struct BodyTask {
    index: usize,
    header: FetchHeader,
    group_order: GroupOrder,
    reading: Reading,
    printed: Arc<Mutex<Printed>>,
    notes: mpsc::Sender<Note>,
}

impl BodyTask {
    /// Reads one fetch stream at the pace the flags ask for, and prints what
    /// it carries.
    ///
    /// The caller drops the body when this is done: a stream that is not at
    /// its end is stopped then.
    async fn read(self, body: &mut FetchBody) {
        tokio::time::sleep(self.reading.after).await;
        let end = self.decode(body).await;
        let _ = self.notes.send(Note::Body(self.index, end)).await;
    }

    async fn decode(&self, body: &mut FetchBody) -> BodyEnd {
        let request_id = self.header.request_id;
        let mut decoder = FetchStreamDecoder::new(FETCH_OBJECT_LIMIT, self.group_order);
        // The readers took the header off; the decoder wants to see one.
        let header_bytes = FetchStreamEncoder::new(self.header, self.group_order).take_bytes();
        if let Err(error) = decoder.push(&header_bytes) {
            return BodyEnd::Broken(error);
        }
        loop {
            // Whatever was pushed is decoded before more is taken.
            loop {
                match decoder.next_event() {
                    Ok(Some(FetchEvent::Header(_))) => {}
                    Ok(Some(FetchEvent::Object(object))) => {
                        let objects = {
                            let mut lines = printed(&self.printed);
                            if lines.ended {
                                return BodyEnd::Stopped;
                            }
                            report::fetch_object(request_id, &object);
                            crate::catalog::offer(&object.payload);
                            lines.objects += 1;
                            lines.objects
                        };
                        if self.reading.cancel_after_objects == Some(objects) {
                            return BodyEnd::GaveUp;
                        }
                    }
                    Ok(Some(FetchEvent::EndOfRange { kind, location })) => {
                        let lines = printed(&self.printed);
                        if lines.ended {
                            return BodyEnd::Stopped;
                        }
                        report::fetch_range_end(request_id, kind, location);
                    }
                    Ok(None) => break,
                    Err(error) => return BodyEnd::Broken(error),
                }
            }
            match body.next().await {
                FetchChunk::Bytes(bytes) => {
                    if let Err(error) = decoder.push(&bytes) {
                        return BodyEnd::Broken(error);
                    }
                    tokio::time::sleep(self.reading.every).await;
                }
                FetchChunk::End(StreamEnd::Fin) => {
                    return match decoder.finish() {
                        Ok(()) => BodyEnd::Fin,
                        Err(error) => BodyEnd::Broken(error),
                    };
                }
                FetchChunk::End(StreamEnd::Aborted(code)) => return BodyEnd::Reset(code),
                FetchChunk::End(StreamEnd::Lost) => return BodyEnd::Lost,
                FetchChunk::Stopped => return BodyEnd::Stopped,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIN: Option<StreamEnded> = Some(StreamEnded::Fin);
    const RESET: Option<StreamEnded> = Some(StreamEnded::Reset(Some(0x6)));

    fn fetch(answered: bool, stream: Option<StreamEnded>) -> Progress {
        Progress {
            answered,
            stream,
            over: false,
            gave_up: false,
        }
    }

    /// (the FETCH before: answered, and its stream; what happens; what the
    ///  session hears; how the FETCH ends, if it does: `how`, `code`, whether
    ///  the request stream is reset; answered and the stream after)
    type Case = (
        (bool, Option<StreamEnded>),
        Happened,
        Option<Tell>,
        Option<(&'static str, Option<u32>, bool)>,
        (bool, Option<StreamEnded>),
    );

    fn check(cases: &[Case]) {
        for ((answered, stream), happened, tell, end, after) in cases.iter().copied() {
            let before = fetch(answered, stream);
            let step = step(before, happened);
            let ended = step
                .ended
                .map(|ended| (ended.how, ended.code, ended.cancel));
            assert_eq!(step.tell, tell, "{before:?} {happened:?}");
            assert_eq!(ended, end, "{before:?} {happened:?}");
            assert_eq!(
                (step.progress.answered, step.progress.stream),
                after,
                "{before:?} {happened:?}"
            );
            // Over exactly where it ended.
            assert_eq!(step.progress.over, end.is_some(), "{before:?} {happened:?}");
            // What the peer still says is the session's to judge, but for a
            // FETCH this client gave up by resetting its request stream:
            // an answer that crosses the reset is no fault of the peer's.
            let gave_up = end.is_some_and(|(_, _, reset)| reset);
            assert_eq!(
                step.progress.hears_the_peer(),
                !gave_up,
                "{before:?} {happened:?}"
            );
        }
    }

    #[test]
    fn a_fetch_is_served_by_its_answer_and_the_end_of_its_stream_in_either_order() {
        let fin = Happened::StreamEnded(StreamEnded::Fin);
        let reset = Happened::StreamEnded(StreamEnded::Reset(Some(0x6)));
        let closed = Some(Tell::StreamClosed);
        check(&[
            ((false, None), Happened::Answered, None, None, (true, None)),
            ((false, None), fin, closed, None, (false, FIN)),
            (
                (false, FIN),
                Happened::Answered,
                None,
                Some(("fin", None, false)),
                (true, FIN),
            ),
            (
                (true, None),
                fin,
                closed,
                Some(("fin", None, false)),
                (true, FIN),
            ),
            // A reset of the fetch stream ends it like a FIN: after the
            // answer the FETCH is over, before it the answer is waited for.
            (
                (true, None),
                reset,
                closed,
                Some(("stream_reset", Some(0x6), false)),
                (true, RESET),
            ),
            ((false, None), reset, closed, None, (false, RESET)),
            (
                (false, RESET),
                Happened::Answered,
                None,
                Some(("stream_reset", Some(0x6), false)),
                (true, RESET),
            ),
            // The REQUEST_ERROR that follows a reset says why.
            (
                (false, RESET),
                Happened::Refused,
                None,
                Some(("refused", None, false)),
                (false, RESET),
            ),
        ]);
    }

    #[test]
    fn what_the_request_stream_of_a_fetch_does_to_it() {
        let closed = |how| Happened::RequestClosed(how);
        let cancel = Some(Tell::Cancel);
        check(&[
            // A FIN: nothing after the answer; before it the FETCH is over,
            // and this client finishes its own direction.
            (
                (true, None),
                closed(RequestClose::Fin),
                None,
                None,
                (true, None),
            ),
            (
                (false, None),
                closed(RequestClose::Fin),
                cancel,
                Some(("request_finished_unanswered", None, false)),
                (false, None),
            ),
            (
                (false, FIN),
                closed(RequestClose::Fin),
                cancel,
                Some(("request_finished_unanswered", None, false)),
                (false, FIN),
            ),
            // A reset, whenever: over, and this client resets its own.
            (
                (true, None),
                closed(RequestClose::Reset(Some(0x5))),
                cancel,
                Some(("request_reset", Some(0x5), true)),
                (true, None),
            ),
            (
                (false, None),
                closed(RequestClose::Reset(None)),
                cancel,
                Some(("request_reset", None, true)),
                (false, None),
            ),
            // A STOP_SENDING for this client's direction: nothing, whenever.
            (
                (false, None),
                closed(RequestClose::Stopped(Some(0x1))),
                None,
                None,
                (false, None),
            ),
            (
                (true, None),
                closed(RequestClose::Stopped(None)),
                None,
                None,
                (true, None),
            ),
            // The answers that end it.
            (
                (false, None),
                Happened::Refused,
                None,
                Some(("refused", None, false)),
                (false, None),
            ),
            (
                (false, None),
                Happened::UnsupportedTrackProperty,
                None,
                Some(("unsupported_track_property", None, true)),
                (false, None),
            ),
        ]);
    }

    #[test]
    fn what_the_client_gives_up_itself() {
        let cancel = Some(Tell::Cancel);
        check(&[
            (
                (false, None),
                Happened::StreamStopped,
                cancel,
                Some(("stream_stopped", None, true)),
                (false, None),
            ),
            (
                (true, None),
                Happened::GaveUp,
                cancel,
                Some(("cancelled", None, true)),
                (true, None),
            ),
            (
                (true, None),
                Happened::StreamRefused,
                cancel,
                Some(("stream_refused", None, true)),
                (true, None),
            ),
            (
                (true, None),
                Happened::Left,
                cancel,
                Some(("left", None, true)),
                (true, None),
            ),
            (
                (false, FIN),
                Happened::Left,
                cancel,
                Some(("left", None, true)),
                (false, FIN),
            ),
            (
                (false, None),
                Happened::NotAnsweredInTime,
                cancel,
                Some(("no_answer_in_time", None, true)),
                (false, None),
            ),
            // Its stream came before its answer: the answer is still owed.
            (
                (false, FIN),
                Happened::NotAnsweredInTime,
                cancel,
                Some(("no_answer_in_time", None, true)),
                (false, FIN),
            ),
            // Answered: the time was for the answer, not for the Objects.
            (
                (true, None),
                Happened::NotAnsweredInTime,
                None,
                None,
                (true, None),
            ),
        ]);
    }

    #[test]
    fn a_fetch_that_is_over_ends_once() {
        let over = Progress {
            over: true,
            ..fetch(true, FIN)
        };
        for happened in [
            Happened::Answered,
            Happened::Refused,
            Happened::RequestClosed(RequestClose::Reset(None)),
            Happened::StreamEnded(StreamEnded::Fin),
            Happened::StreamEnded(StreamEnded::Reset(None)),
            Happened::GaveUp,
            Happened::Left,
            Happened::NotAnsweredInTime,
        ] {
            let step = step(over, happened);
            assert_eq!(
                (step.progress, step.tell, step.ended),
                (over, None, None),
                "{happened:?}"
            );
        }
    }
}
