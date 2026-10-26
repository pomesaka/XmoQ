//! The streams of a session of a client that serves tracks, each written by
//! a task of its own: the request streams the client opens, and the
//! subgroup stream of each Group it publishes. The answer to each FETCH the
//! relay sends it is [`crate::answers`].
//!
//! WHY tasks: a relay that takes nothing on one stream must not hold up the
//! client, which goes on with every other track. The session
//! (`xmoq_moqt::session::Session`) stays with the client's loop; these tasks
//! say what happened, and are told what to send.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, RequestClose, RequestReadError, RequestReader, RequestStreamEvent, SendStream,
    Trace, WriteError, send_message,
};

use crate::answers::PEER_END_WAIT;

type Send = <WtConnection as Connection>::Send;

/// Messages for one request stream that its task has not sent yet: among
/// them the answers to the relay's `REQUEST_UPDATE`s, of which §10.9 has
/// "MUST respond with exactly one `REQUEST_OK` or `REQUEST_ERROR`". None is
/// dropped: where one more does not fit, the client's loop ends the request
/// instead, with `EXCESSIVE_LOAD` (`tell` of `crates/chat/src/bin/xmoq-chat/session.rs`). WHY 64: what the own relay
/// holds of the updates of one FETCH that wait for their answer
/// (`MAX_HELD_FETCH_UPDATES`), before it ends that FETCH the same way.
/// **Provisional.**
pub const COMMAND_QUEUE: usize = 64;

/// How long the Object on a subgroup stream may wait for the relay to take
/// its bytes; then the stream is reset with `CANCELLED`. WHY a limit: the
/// `PUBLISH_DONE` of a track waits for every stream of it to be closed
/// (§10.11), and a relay that takes nothing would hold it for good.
/// **Provisional.**
const GROUP_WRITE_WAIT: Duration = Duration::from_secs(5);

/// How long the subgroup stream of a Group may wait for the relay to allow
/// one more stream. Then the Group is not sent live; the publishers here
/// keep it, and a FETCH gets it. **Provisional**: long against the moment a
/// stream takes where there is room, short against a person waiting for a
/// line.
const GROUP_OPEN_WAIT: Duration = Duration::from_secs(1);

/// What the loop tells the task of one of its request streams.
#[derive(Debug)]
pub enum Command {
    /// Send this message.
    Say(ControlMessage),
    /// Finish the client's direction, and wait [`PEER_END_WAIT`] for the
    /// relay to end its own.
    Finish,
    /// Reset the client's direction and stop the relay's with this code
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
    /// The relay sent `STOP_SENDING` for the client's direction.
    Stopped(Option<u32>),
    /// The relay's bytes break the protocol, or the connection went.
    Failed(RequestReadError),
}

/// The waits of a request: for its stream, and for its first answer.
#[derive(Debug, Clone, Copy)]
pub struct Waits {
    /// For the relay to allow the stream.
    pub open: Duration,
    /// For the first message of the relay on it.
    pub answer: Duration,
}

/// What the client's loop keeps of the task of one of its request streams.
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
/// is dropped. After the client finished its direction, the relay gets
/// [`PEER_END_WAIT`] to end its own before it is stopped.
// FIXME: generic over `Connection`, as `crate::answers` is, with
// `send_group` and `PeerRequests`, so that tests on `MemoryConnection`
// reach them.
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

/// Cancels a request on its stream (§3.3.2): a reset of the client's
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

/// A Group of one Object to send on its own subgroup stream: Object 0 of
/// Subgroup 0, as a line of the chat is sent (decided on 2026-10-24).
pub struct GroupToSend {
    /// The Track Alias of the PUBLISH.
    pub track_alias: u64,
    /// The Group ID.
    pub group_id: u64,
    /// The payload of its Object 0.
    pub payload: Vec<u8>,
    /// Counts the streams of the subscription that were opened, for its
    /// `PUBLISH_DONE` (§10.11: Stream Count).
    pub opened: Arc<AtomicU64>,
}

/// Sends a Group. What becomes of it is no more than a word for the log:
/// the publishers that use it keep the Group either way, and a FETCH gets
/// it.
///
/// # Errors
///
/// Why it was not sent, for the log.
pub async fn send_group(connection: Arc<WtConnection>, group: GroupToSend) -> Result<(), String> {
    let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
        track_alias: group.track_alias,
        group_id: group.group_id,
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
            payload: group.payload,
        })
        .map_err(|error| error.to_string())?;
    let mut data = match tokio::time::timeout(GROUP_OPEN_WAIT, connection.open_uni()).await {
        Ok(Ok(data)) => data,
        Ok(Err(lost)) => return Err(lost.to_string()),
        Err(_) => {
            return Err(format!(
                "no stream within {} ms",
                GROUP_OPEN_WAIT.as_millis()
            ));
        }
    };
    group.opened.fetch_add(1, Ordering::Relaxed);
    let written =
        tokio::time::timeout(GROUP_WRITE_WAIT, data.write_all(&encoder.take_bytes())).await;
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
                GROUP_WRITE_WAIT.as_millis()
            ))
        }
    }
}

/// Counts a Group being sent on a stream of its track until it is dropped,
/// also where its task is ended from outside: what the `PUBLISH_DONE` of
/// the track waits for (§10.11).
pub struct Sending(tokio::sync::watch::Sender<usize>);

impl Sending {
    /// One more Group of the track is being sent.
    #[must_use]
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
