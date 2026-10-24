//! The streams of the session, each written by a task of its own: the
//! request streams the gatherer opens, the subgroup stream of each line it
//! publishes, and the answer to each FETCH the relay sends it.
//!
//! WHY tasks: a relay that takes nothing on one stream must not hold up the
//! gatherer, which goes on taking the lines of everybody else. The session
//! (`xmoq_moqt::session::Session`) stays with the loop of `session.rs`;
//! these tasks say what happened, and are told what to send.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::fetch::{FetchHeader, FetchObject, FetchStreamEncoder, RangeEnd};
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, RequestClose, RequestReadError, RequestReader, RequestStreamEvent, SendStream,
    Trace, WriteError, open_fetch_stream, send_message,
};

type Send = <WtConnection as Connection>::Send;

/// Messages for one request stream that its task has not sent yet: among
/// them the answers to the relay's `REQUEST_UPDATE`s, of which §10.9 has
/// "MUST respond with exactly one `REQUEST_OK` or `REQUEST_ERROR`". None is
/// dropped: where one more does not fit, the loop ends the request instead,
/// with `EXCESSIVE_LOAD` (`session.rs`, `tell`). WHY 64: what the own relay
/// holds of the updates of one FETCH that wait for their answer
/// (`MAX_HELD_FETCH_UPDATES`), before it ends that FETCH the same way.
/// **Provisional.**
pub const COMMAND_QUEUE: usize = 64;

/// How long the relay gets to end its direction of a request stream after
/// the gatherer finished its own, before it is stopped. The time the Rust
/// client and the relay wait (`PEER_END_WAIT` of
/// `crates/moqt-client/src/serving.rs`). Provisional.
pub const PEER_END_WAIT: Duration = Duration::from_secs(3);

/// How long the line on a subgroup stream may wait for the relay to take
/// its bytes; then the stream is reset with `CANCELLED`. WHY a limit: the
/// `PUBLISH_DONE` of a room waits for every stream of the room to be closed
/// (§10.11), and a relay that takes nothing would hold it for good.
/// **Provisional.**
const LINE_WRITE_WAIT: Duration = Duration::from_secs(5);

/// How long the subgroup stream of a line may wait for the relay to allow
/// one more stream. Then the line is not sent live; it is in the store, and
/// a FETCH gets it. **Provisional**: long against the moment a stream takes
/// where there is room, short against a person waiting for a line.
const LINE_OPEN_WAIT: Duration = Duration::from_secs(1);

/// What the loop tells the task of one of its request streams.
#[derive(Debug)]
pub enum Command {
    /// Send this message.
    Say(ControlMessage),
    /// Finish the gatherer's direction, and wait [`PEER_END_WAIT`] for the
    /// relay to end its own.
    Finish,
    /// Reset the gatherer's direction and stop the relay's with this code
    /// (§3.3.2: a request is cancelled so).
    Cancel(u32),
}

/// What the task of a request stream says happened.
#[derive(Debug)]
pub enum Heard {
    /// The stream is open, and the request was written.
    Opened,
    /// It was not: no stream in time, or the connection went.
    NotOpened(String),
    /// A message of the relay.
    Message(ControlMessage),
    /// No message came within the time the request was given.
    NotAnswered,
    /// The relay's direction is over.
    Closed(RequestClose),
    /// The relay sent `STOP_SENDING` for the gatherer's direction.
    Stopped(Option<u32>),
    /// The relay's bytes break the protocol, or the connection went.
    Failed(RequestReadError),
}

/// The waits of a request: for its stream, and for its first answer.
#[derive(Debug, Clone, Copy)]
pub struct Waits {
    pub open: Duration,
    pub answer: Duration,
}

/// What the loop keeps of the task of one of its request streams.
pub struct RequestTask {
    /// What it is told, in order.
    pub commands: mpsc::Sender<Command>,
    /// Cancels the request with this code before anything that waits in
    /// `commands`.
    pub abort: Option<oneshot::Sender<u32>>,
}

/// Opens a request stream, writes `message` on it, and keeps reading it and
/// writing what it is told, until the stream is over. What happens is sent
/// to `heard` with `token`. The stream is cancelled when what this returns
/// is dropped.
pub fn open_request(
    connection: &Arc<WtConnection>,
    token: u64,
    message: ControlMessage,
    waits: Waits,
    heard: mpsc::Sender<(u64, Heard)>,
) -> RequestTask {
    let (commands, told) = mpsc::channel(COMMAND_QUEUE);
    let (abort, aborted) = oneshot::channel();
    tokio::spawn(request(
        Arc::clone(connection),
        token,
        message,
        waits,
        heard,
        (told, aborted),
    ));
    RequestTask {
        commands,
        abort: Some(abort),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one stream's whole life in one select: what it is told, what it reads, its times"
)]
async fn request(
    connection: Arc<WtConnection>,
    token: u64,
    message: ControlMessage,
    waits: Waits,
    heard: mpsc::Sender<(u64, Heard)>,
    (mut told, mut aborted): (mpsc::Receiver<Command>, oneshot::Receiver<u32>),
) {
    let report = |what| {
        let heard = heard.clone();
        async move {
            // The loop is gone when this fails, and the session with it.
            let _ = heard.send((token, what)).await;
        }
    };
    let label = format!("request#{token}");
    let trace = Trace::default();
    let (mut send, recv) = match tokio::time::timeout(waits.open, connection.open_bi()).await {
        Ok(Ok(halves)) => halves,
        Ok(Err(lost)) => return report(Heard::NotOpened(lost.to_string())).await,
        Err(_) => {
            let why = format!("no stream within {} ms", waits.open.as_millis());
            return report(Heard::NotOpened(why)).await;
        }
    };
    if let Err(error) = send_message(&mut send, &message, &trace, &label).await {
        send.reset(stream_reset_code::CANCELLED);
        recv_stop(recv);
        return report(Heard::NotOpened(error.to_string())).await;
    }
    let mut reader = RequestReader::new(recv, &trace, &label);
    report(Heard::Opened).await;
    let mut send = Some(send);
    let answer_by = tokio::time::sleep(waits.answer);
    tokio::pin!(answer_by);
    let mut answered = false;
    // Whether the relay's direction is over, and nothing more is read.
    let mut peer_done = false;
    // Whether a STOP_SENDING is still to be told of.
    let mut watching = true;
    loop {
        tokio::select! {
            biased;
            // Dropped without a code: the loop let go of the request, and
            // `told` ends as well.
            code = &mut aborted, if !aborted.is_terminated() => {
                if let Ok(code) = code {
                    return cancel(send, reader, peer_done, code);
                }
            }
            command = told.recv() => match command {
                Some(Command::Say(message)) => {
                    if let Some(send) = send.as_mut() {
                        // A failure is a STOP_SENDING or a lost connection,
                        // which the branches below and the session hear of.
                        let _ = send_message(send, &message, &trace, &label).await;
                    }
                }
                Some(Command::Finish) => {
                    if let Some(send) = send.take() {
                        send.finish();
                    }
                    if !peer_done {
                        let closed = async {
                            loop {
                                match reader.next_event().await {
                                    Ok(RequestStreamEvent::Message(_)) => {}
                                    Ok(RequestStreamEvent::Closed(how)) => return Some(how),
                                    Err(_) => return None,
                                }
                            }
                        };
                        match tokio::time::timeout(PEER_END_WAIT, closed).await {
                            Ok(Some(how)) => report(Heard::Closed(how)).await,
                            Ok(None) => report(Heard::Closed(RequestClose::Reset(None))).await,
                            Err(_) => {
                                reader.stop(stream_reset_code::CANCELLED);
                                return report(Heard::Closed(RequestClose::Reset(None))).await;
                            }
                        }
                    }
                    return;
                }
                Some(Command::Cancel(code)) => return cancel(send, reader, peer_done, code),
                // The loop let go of the request: as a cancel.
                None => return cancel(send, reader, peer_done, stream_reset_code::CANCELLED),
            },
            event = reader.next_event(), if !peer_done => match event {
                Ok(RequestStreamEvent::Message(message)) => {
                    answered = true;
                    report(Heard::Message(message)).await;
                }
                Ok(RequestStreamEvent::Closed(how)) => {
                    peer_done = true;
                    report(Heard::Closed(how)).await;
                }
                Err(error) => {
                    if let Some(send) = send.take() {
                        send.reset(stream_reset_code::CANCELLED);
                    }
                    return report(Heard::Failed(error)).await;
                }
            },
            () = &mut answer_by, if !answered => {
                answered = true;
                report(Heard::NotAnswered).await;
            }
            stopped = stopped(send.as_mut()), if watching => {
                watching = false;
                if let WriteError::Stopped(code) = stopped {
                    report(Heard::Stopped(code)).await;
                }
            }
        }
    }
}

/// Cancels a request on its stream (§3.3.2): a reset of the gatherer's
/// direction, and `STOP_SENDING` for the relay's where it is not over.
fn cancel(
    send: Option<Send>,
    reader: RequestReader<<WtConnection as Connection>::Recv>,
    peer_done: bool,
    code: u32,
) {
    if let Some(send) = send {
        send.reset(code);
    }
    if !peer_done {
        reader.stop(code);
    }
}

/// Stops the receiving half of a stream that was never read.
fn recv_stop(recv: <WtConnection as Connection>::Recv) {
    use xmoq_moqt_io::RecvStream;
    recv.stop(stream_reset_code::CANCELLED);
}

/// [`SendStream::stopped`] of a half that may be gone.
async fn stopped(send: Option<&mut Send>) -> WriteError {
    match send {
        Some(send) => send.stopped().await,
        None => std::future::pending().await,
    }
}

/// A line to send on its own subgroup stream: Object 0 of the Group whose
/// ID is its number (decided on 2026-10-24).
pub struct LineToSend {
    pub track_alias: u64,
    pub number: u64,
    pub payload: Vec<u8>,
    /// Counts the streams of the subscription that were opened, for its
    /// `PUBLISH_DONE` (§10.11: Stream Count).
    pub opened: Arc<AtomicU64>,
}

/// Sends a line. What becomes of it is no more than a word for the log:
/// the line is in the store either way, and a FETCH gets it.
pub async fn send_line(connection: Arc<WtConnection>, line: LineToSend) -> Result<(), String> {
    let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
        track_alias: line.track_alias,
        group_id: line.number,
        subgroup_id: SubgroupId::Zero,
        // Left out: the track's default, 128 (§12.4), which a FETCH says too.
        publisher_priority: None,
        properties: false,
        // One Object a Group: the stream has its largest and its first.
        end_of_group: true,
        first_object: true,
    });
    encoder
        .object(&SubgroupObject {
            object_id: 0,
            properties: vec![],
            status: ObjectStatus::Normal,
            payload: line.payload,
        })
        .map_err(|error| error.to_string())?;
    let mut data = match tokio::time::timeout(LINE_OPEN_WAIT, connection.open_uni()).await {
        Ok(Ok(data)) => data,
        Ok(Err(lost)) => return Err(lost.to_string()),
        Err(_) => {
            return Err(format!(
                "no stream within {} ms",
                LINE_OPEN_WAIT.as_millis()
            ));
        }
    };
    line.opened.fetch_add(1, Ordering::Relaxed);
    let written =
        tokio::time::timeout(LINE_WRITE_WAIT, data.write_all(&encoder.take_bytes())).await;
    match written {
        // §11.4.3: every Object of the Subgroup was written, so a FIN.
        Ok(Ok(())) => {
            data.finish();
            Ok(())
        }
        // §11.4.3: "The publisher SHOULD respond with a reset."
        Ok(Err(WriteError::Stopped(code))) => {
            data.reset(stream_reset_code::CANCELLED);
            Err(format!("stopped with {code:?}"))
        }
        Ok(Err(WriteError::Lost(why))) => Err(why),
        Err(_) => {
            data.reset(stream_reset_code::CANCELLED);
            Err(format!(
                "not taken within {} ms",
                LINE_WRITE_WAIT.as_millis()
            ))
        }
    }
}

/// Counts a line being sent on a stream of its room until it is dropped,
/// also where its task is ended from outside: what the `PUBLISH_DONE` of
/// the room waits for (§10.11).
pub struct Sending(pub tokio::sync::watch::Sender<usize>);

impl Sending {
    /// One more line of the room is being sent.
    pub fn begin(count: &tokio::sync::watch::Sender<usize>) -> Self {
        count.send_modify(|count| *count += 1);
        Self(count.clone())
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// What the answer to a FETCH carries after its header, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Object(FetchObject),
    EndOfRange(RangeEnd, Location),
}

/// A FETCH that was accepted, to be answered on its streams.
pub struct Answer {
    /// `FETCH_OK`, and a `REQUEST_OK` for every `REQUEST_UPDATE` that came
    /// before it.
    pub messages: Vec<ControlMessage>,
    pub request_id: u64,
    pub group_order: GroupOrder,
    pub pieces: Vec<Piece>,
}

/// How the answer to a FETCH ended, for the loop and the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    /// Its fetch stream was written to its FIN.
    Written,
    /// The relay stopped the fetch stream or the request stream.
    Stopped,
    /// The fetch stream could not be opened or written.
    Failed,
    /// After [`Self::Written`]: the gatherer's direction of the request
    /// stream is over, finished or, where the relay stopped it meanwhile,
    /// reset; `peer_ended` says whether the relay's direction had ended by
    /// then.
    RequestOver { peer_ended: bool },
}

/// What the loop keeps of a FETCH that is answered.
pub struct Answering {
    /// Messages for the request stream: the `REQUEST_OK` of a
    /// `REQUEST_UPDATE` (§10.9). Where one does not fit, the loop cancels
    /// the FETCH ([`COMMAND_QUEUE`]).
    pub says: mpsc::Sender<ControlMessage>,
    /// Sent when the relay's direction of the request stream is over: the
    /// task finishes its own then, or after [`PEER_END_WAIT`].
    pub peer_ended: Option<oneshot::Sender<()>>,
    /// Ends the answer, both streams reset with this code (§5.2: "It MUST
    /// reset the bidi request stream and unidirectional data stream
    /// associated with the FETCH"). Dropped, the same with `CANCELLED`.
    pub cancel: Option<oneshot::Sender<u32>>,
}

/// Answers a FETCH on the request stream `send` and a fetch stream of its
/// own: the messages, then the pieces, then the FIN of the fetch stream,
/// then that of the request stream once the relay has ended its direction
/// or [`PEER_END_WAIT`] is over. What happened goes to `done` with `stream`:
/// [`Answered::Written`] and then [`Answered::RequestOver`], or one of the
/// others.
///
/// WHY the request stream waits for the relay: §5.2 has a FETCH end with
/// its fetch stream, and says nothing of a FIN on the request stream; the
/// two arrive apart, and a subscriber may take a finished request stream for
/// the end of the FETCH before it has read the fetch stream. The Rust
/// client answers the same way (`crates/moqt-client/src/serving.rs`), and the
/// row on the request stream of a FETCH that was served to its end in
/// `docs/moqt-implementation-notes.md` has the rest.
#[expect(
    clippy::too_many_lines,
    reason = "both streams of one FETCH in one task, in the order they are written and ended"
)]
pub fn answer_fetch(
    connection: &Arc<WtConnection>,
    stream: u64,
    mut send: Send,
    answer: Answer,
    done: mpsc::Sender<(u64, Answered)>,
) -> Answering {
    let (says, mut said) = mpsc::channel::<ControlMessage>(COMMAND_QUEUE);
    let (peer_ended, mut peer_end) = oneshot::channel();
    let (cancel, mut cancelled) = oneshot::channel::<u32>();
    let connection = Arc::clone(connection);
    tokio::spawn(async move {
        let label = format!("bi#{stream}");
        let trace = Trace::default();
        let Answer {
            messages,
            request_id,
            group_order,
            pieces,
        } = answer;
        let work = async {
            for message in &messages {
                if send_message(&mut send, message, &trace, &label)
                    .await
                    .is_err()
                {
                    return Answered::Stopped;
                }
            }
            let header = FetchHeader { request_id };
            let fetch_label = format!("fetch#{request_id}");
            let Ok(mut writer) = open_fetch_stream(
                &*connection,
                header,
                &trace,
                &fetch_label,
                stream_reset_code::CANCELLED,
            )
            .await
            else {
                return Answered::Failed;
            };
            let mut encoder = FetchStreamEncoder::new(header, group_order);
            // The header, which `open_fetch_stream` has written.
            encoder.take_bytes();
            // Taken one by one: what was written is let go of, and the bytes
            // the loop counts for the answer are gone when it is written.
            for piece in pieces {
                match piece {
                    Piece::Object(object) => {
                        if encoder.object(&object).is_err() {
                            // An Object that cannot be encoded is the
                            // gatherer's mistake: the stream cannot go on.
                            writer.reset(stream_reset_code::INTERNAL_ERROR);
                            return Answered::Failed;
                        }
                    }
                    Piece::EndOfRange(kind, location) => encoder.end_of_range(kind, location),
                }
                match writer.write(&encoder.take_bytes()).await {
                    Ok(()) => {}
                    Err(WriteError::Stopped(_)) => return Answered::Stopped,
                    Err(WriteError::Lost(_)) => return Answered::Failed,
                }
            }
            writer.finish();
            Answered::Written
        };
        let answered = tokio::select! {
            answered = work => answered,
            code = &mut cancelled => {
                send.reset(code.unwrap_or(stream_reset_code::CANCELLED));
                return;
            }
        };
        // The loop is gone when this fails.
        let _ = done.send((stream, answered)).await;
        if answered != Answered::Written {
            send.reset(stream_reset_code::CANCELLED);
            return;
        }
        let waited = tokio::time::sleep(PEER_END_WAIT);
        tokio::pin!(waited);
        let (mut peer_ended, mut stopped, mut said_more) = (false, false, true);
        loop {
            tokio::select! {
                biased;
                code = &mut cancelled, if !cancelled.is_terminated() => {
                    if let Ok(code) = code {
                        send.reset(code);
                        return;
                    }
                }
                // A REQUEST_OK that came while the answer was written goes
                // out before the FIN.
                message = said.recv(), if said_more => match message {
                    Some(message) => {
                        let _ = send_message(&mut send, &message, &trace, &label).await;
                    }
                    None => said_more = false,
                },
                ended = &mut peer_end, if !peer_end.is_terminated() => {
                    peer_ended = ended.is_ok();
                    if peer_ended {
                        break;
                    }
                }
                // The row of the notes: a STOP_SENDING during the wait has
                // this direction reset with CANCELLED.
                gone = send.stopped() => {
                    // Lost: the connection is gone, and the stream with it.
                    stopped = matches!(gone, WriteError::Stopped(_));
                    break;
                }
                () = &mut waited => break,
            }
        }
        if stopped {
            send.reset(stream_reset_code::CANCELLED);
        } else {
            send.finish();
        }
        let _ = done
            .send((stream, Answered::RequestOver { peer_ended }))
            .await;
    });
    Answering {
        says,
        peer_ended: Some(peer_ended),
        cancel: Some(cancel),
    }
}
