//! Connects the sessions of the relay to its one decision maker.
//!
//! Every session has readers that turn its streams into [`Event`]s. One task,
//! the hub, feeds them to the [`Relay`] in the order they arrive and carries out
//! the [`Action`]s it returns. The hub never waits for a peer and never writes
//! to anything that can block: what it hands to a session goes into a bounded
//! queue, and a queue that is full ends that session or that subscription
//! instead of holding up the others.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use xmoq_moqt::data::SubgroupHeader;
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::session::MAX_PEER_REQUESTS;
use xmoq_moqt::{TerminationCode, stream_reset_code};
use xmoq_moqt_io::{
    Connection, IncomingStreams, OutgoingEvent, OutgoingSubgroup, PeerRequestStreamEvent,
    SendBudget, SendLimits, SendStream, StreamConfig, StreamEnd, StreamStops, Trace, UniEvent,
    WriteError, read_request_streams, read_uni_streams, send_message,
};
use xmoq_relay_core::{Action, Event, OutStreamKey, Relay, Role, SessionKey, StreamKey};

/// Limits of the relay. Every value is provisional: the draft sets none of
/// them, and `XmoQ` has not decided any.
///
/// What one session can make the relay hold, with the defaults here and the
/// QUIC limits the relay is started with (`quic_limits` in `main.rs`):
///
/// | Held | Bound |
/// |---|---|
/// | QUIC: received on its streams and not read yet | 4 MiB (`receive_window`) |
/// | QUIC: written to its streams and not acknowledged | 4 MiB (`send_window`) |
/// | Read buffers of its streams | 64 + 264 streams of 16 KiB: about 5 MiB |
/// | Undecoded bytes of its subgroup streams | 4 MiB (`session_buffer_limit`) |
/// | Streams held for a Track Alias that is not known | 1 MiB (`MAX_EARLY_BYTES`) |
/// | Objects waiting to be written to it | 4 MiB (`send.bytes`) |
/// | Messages waiting to be written to it | 256 a stream |
///
/// That is about 22 MiB a session and 5.6 GiB for 256 sessions, plus what the
/// events that wait for the hub hold (shared, 1024 events). Without the QUIC
/// limits the first two rows are those of the transport library, which for
/// quinn's defaults are far larger than everything else here.
#[derive(Clone)]
pub struct HubConfig {
    /// Limits and deadlines of the streams of each session.
    pub streams: StreamConfig,
    /// Sessions served at once. One more is closed at once.
    pub max_sessions: usize,
    /// Events from all sessions that wait for the hub. Readers wait beyond
    /// that, which holds their peers back through flow control.
    pub event_queue_len: usize,
    /// Control messages that wait to be written to one session, and messages
    /// that wait on one of its request streams. A session that lets them pile
    /// up further is closed.
    pub session_queue_len: usize,
    /// What may wait to be written to one subscriber, over all its subgroup
    /// streams. A subscriber that lets more pile up loses the subscription
    /// that hit the limit (`TOO_FAR_BEHIND`).
    pub send: SendLimits,
    /// How long the last Objects of a track are waited for after
    /// `PUBLISH_DONE` (§10.11 of `draft-ietf-moq-transport-18`).
    pub drain_timeout: Duration,
    /// How long a stream is held for a Track Alias that is not known yet
    /// (§11.4.2: "buffer it for a brief period").
    pub early_stream_timeout: Duration,
    /// How long a peer gets to close its direction of a request stream after
    /// the relay ended the request with `PUBLISH_DONE`. Then the relay stops
    /// the stream.
    pub request_close_timeout: Duration,
    /// How long the relay goes on reading what had arrived on a connection
    /// after the connection is gone, before it treats the session as closed.
    pub closed_read_timeout: Duration,
    /// Sent as `MOQT_IMPLEMENTATION`.
    pub implementation: String,
    /// Where one line about each thing the relay does goes, if anywhere.
    pub log: Option<Log>,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            streams: StreamConfig {
                // WHY above the session's limit on open requests: a request
                // over that limit has to be read to be answered with
                // EXCESSIVE_LOAD. With fewer readers than requests it would
                // wait unanswered instead.
                max_request_streams: MAX_PEER_REQUESTS + 8,
                ..StreamConfig::default()
            },
            max_sessions: 256,
            event_queue_len: 1024,
            session_queue_len: 256,
            send: SendLimits::default(),
            drain_timeout: Duration::from_secs(2),
            early_stream_timeout: Duration::from_secs(2),
            request_close_timeout: Duration::from_secs(2),
            closed_read_timeout: Duration::from_secs(1),
            implementation: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))
                .to_string(),
            log: None,
        }
    }
}

/// Takes log lines without ever making the caller wait.
#[derive(Clone)]
pub struct Log {
    lines: mpsc::Sender<String>,
    /// Lines that did not fit in the queue since the last one that did.
    dropped: Arc<AtomicU64>,
}

impl Log {
    /// A log that prints to stdout from a task of its own.
    ///
    /// WHY a task: the hub is one task for all sessions, and a write to a
    /// stdout that nobody reads would stop it, and all sessions with it. Lines
    /// that do not fit in the queue are dropped, and their number is printed
    /// when there is room again.
    #[must_use]
    pub fn stdout() -> Self {
        const QUEUE: usize = 1024;
        let (lines, mut queued) = mpsc::channel::<String>(QUEUE);
        tokio::spawn(async move {
            while let Some(line) = queued.recv().await {
                println!("{line}");
            }
        });
        Self {
            lines,
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Queues a line. A line that does not fit is dropped: the log is not
    /// worth a stalled relay.
    pub fn print(&self, line: String) {
        // Relaxed: a count for a log line.
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            let notice = format!("LOG_DROPPED lines={dropped}");
            if self.lines.try_send(notice).is_err() {
                self.dropped.fetch_add(dropped, Ordering::Relaxed);
            }
        }
        if self.lines.try_send(line).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// What reaches the hub.
enum Envelope<C: Connection> {
    NewSession {
        session: SessionKey,
        connection: Arc<C>,
    },
    /// The first message of a request stream, with the half to answer on.
    RequestOpened {
        session: SessionKey,
        stream: StreamKey,
        send: C::Send,
        first: ControlMessage,
    },
    Core(Event),
    /// The session cannot go on, for a reason found on the I/O side.
    Broken {
        session: SessionKey,
        code: TerminationCode,
        reason: &'static str,
    },
    /// A test asks what the hub holds.
    #[cfg(test)]
    Inspect(tokio::sync::oneshot::Sender<Held>),
}

/// What the hub and its relay hold.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    /// Sessions the hub has handles for.
    pub sessions: usize,
    /// Writers of request streams, over all sessions.
    pub request_writers: usize,
    /// Writers of subgroup streams that are not done.
    pub out_streams: usize,
    /// What the relay holds.
    pub relay: xmoq_relay_core::Counts,
}

/// What the hub asks the writer of one request stream to do.
enum RequestCommand {
    Send(ControlMessage),
    Finish,
    Reset(u32),
}

struct SessionHandle<C: Connection> {
    connection: Arc<C>,
    control: mpsc::Sender<ControlMessage>,
    /// The writers of the request streams the peer has open.
    requests: HashMap<StreamKey, mpsc::Sender<RequestCommand>>,
    stops: StreamStops,
    /// What waits to be written to this session's subgroup streams.
    budget: Arc<SendBudget>,
    /// The writers of its control and request streams, and the task that owns
    /// its readers.
    tasks: Vec<JoinHandle<()>>,
}

impl<C: Connection> Drop for SessionHandle<C> {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Adds sessions to a running hub.
pub struct HubHandle<C: Connection> {
    events: mpsc::Sender<Envelope<C>>,
    next_session: Arc<AtomicU64>,
}

impl<C: Connection> Clone for HubHandle<C> {
    fn clone(&self) -> Self {
        Self {
            events: self.events.clone(),
            next_session: Arc::clone(&self.next_session),
        }
    }
}

impl<C: Connection> HubHandle<C> {
    /// What the hub holds, once it has dealt with everything queued before.
    #[cfg(test)]
    pub async fn held(&self) -> Held {
        let (answer, held) = tokio::sync::oneshot::channel();
        let asked = self.events.send(Envelope::Inspect(answer)).await;
        assert!(asked.is_ok(), "the hub is running");
        held.await.expect("the hub answers")
    }

    /// Hands a connection whose handshake is done to the relay.
    pub async fn add_session(&self, connection: C) {
        // Relaxed: the numbers only have to differ.
        let session = SessionKey(self.next_session.fetch_add(1, Ordering::Relaxed));
        let envelope = Envelope::NewSession {
            session,
            connection: Arc::new(connection),
        };
        // An error means the hub is gone, and the connection with the envelope.
        let _ = self.events.send(envelope).await;
    }
}

/// Starts the hub of a relay. The returned task ends only if the hub panics,
/// since handles to it are held by the hub itself; whoever starts the hub
/// watches the task and does not go on accepting sessions without it.
#[must_use]
pub fn spawn<C: Connection>(config: HubConfig) -> (HubHandle<C>, JoinHandle<()>) {
    let (events, inbox) = mpsc::channel(config.event_queue_len);
    let hub = Hub {
        relay: Relay::new(&config.implementation),
        config,
        events: events.clone(),
        sessions: HashMap::new(),
        out_streams: HashMap::new(),
    };
    let task = tokio::spawn(hub.run(inbox));
    let handle = HubHandle {
        events,
        next_session: Arc::new(AtomicU64::new(0)),
    };
    (handle, task)
}

struct Hub<C: Connection> {
    relay: Relay,
    config: HubConfig,
    /// For the tasks the hub starts, which report back through it.
    events: mpsc::Sender<Envelope<C>>,
    sessions: HashMap<SessionKey, SessionHandle<C>>,
    /// The writers of the subgroup streams that are not done, with the session
    /// each writes to.
    out_streams: HashMap<OutStreamKey, (SessionKey, OutgoingSubgroup)>,
}

impl<C: Connection> Hub<C> {
    async fn run(mut self, mut inbox: mpsc::Receiver<Envelope<C>>) {
        while let Some(envelope) = inbox.recv().await {
            match envelope {
                Envelope::NewSession {
                    session,
                    connection,
                } => self.add_session(session, connection),
                Envelope::RequestOpened {
                    session,
                    stream,
                    send,
                    first,
                } => {
                    self.add_request(session, stream, send);
                    self.feed(VecDeque::from([Event::RequestOpened {
                        session,
                        stream,
                        first,
                    }]));
                }
                Envelope::Core(event) => {
                    match &event {
                        // The peer closed the stream; the relay's half is
                        // closed too. Dropping the queue ends the writer, which
                        // finishes the stream.
                        Event::RequestClosed { session, stream } => {
                            if let Some(handle) = self.sessions.get_mut(session) {
                                handle.requests.remove(stream);
                                handle.stops.stop(stream.0, stream_reset_code::CANCELLED);
                            }
                        }
                        Event::OutStreamClosed { out_stream } => {
                            self.out_streams.remove(out_stream);
                        }
                        _ => {}
                    }
                    self.feed(VecDeque::from([event]));
                }
                Envelope::Broken {
                    session,
                    code,
                    reason,
                } => {
                    let mut pending = VecDeque::new();
                    self.close(session, code.code(), reason, &mut pending);
                    self.feed(pending);
                }
                #[cfg(test)]
                Envelope::Inspect(answer) => {
                    let _ = answer.send(Held {
                        sessions: self.sessions.len(),
                        request_writers: self
                            .sessions
                            .values()
                            .map(|handle| handle.requests.len())
                            .sum(),
                        out_streams: self.out_streams.len(),
                        relay: self.relay.counts(),
                    });
                }
            }
        }
    }

    fn log(&self, line: impl FnOnce() -> String) {
        if let Some(log) = &self.config.log {
            log.print(line());
        }
    }

    fn add_session(&mut self, session: SessionKey, connection: Arc<C>) {
        if self.sessions.len() >= self.config.max_sessions {
            self.log(|| format!("SESSION_REFUSED session={} too many sessions", session.0));
            connection.close(TerminationCode::InternalError.code(), b"too many sessions");
            return;
        }
        self.log(|| format!("SESSION_OPENED session={}", session.0));
        let stops = StreamStops::default();
        let (control, control_inbox) = mpsc::channel(self.config.session_queue_len);

        let uni = read_uni_streams(
            Arc::clone(&connection),
            self.config.streams.clone(),
            stops.clone(),
            self.events.clone(),
            move |event| uni_envelope(session, event),
        );
        let requests = read_request_streams(
            Arc::clone(&connection),
            self.config.streams.clone(),
            stops.clone(),
            self.events.clone(),
            move |event| request_envelope(session, event),
        );
        let writer = tokio::spawn(write_control(
            session,
            Arc::clone(&connection),
            control_inbox,
            self.config.streams.trace.clone(),
            self.events.clone(),
        ));
        let watcher = tokio::spawn(watch_session(
            session,
            Arc::clone(&connection),
            [uni, requests],
            self.config.closed_read_timeout,
            self.events.clone(),
        ));
        self.sessions.insert(
            session,
            SessionHandle {
                connection,
                control,
                requests: HashMap::new(),
                stops,
                budget: SendBudget::new(self.config.send),
                tasks: vec![writer, watcher],
            },
        );
        self.feed(VecDeque::from([Event::SessionOpened {
            session,
            role: Role::Server,
        }]));
    }

    fn add_request(&mut self, session: SessionKey, stream: StreamKey, send: C::Send) {
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        let (commands, command_inbox) = mpsc::channel(self.config.session_queue_len);
        let writer = tokio::spawn(write_request(
            session,
            stream,
            send,
            command_inbox,
            self.config.streams.trace.clone(),
            self.events.clone(),
        ));
        handle.requests.insert(stream, commands);
        // Finished writers are dropped from the list as new ones come, so that
        // it holds about as many as there are open requests.
        handle.tasks.retain(|task| !task.is_finished());
        handle.tasks.push(writer);
    }

    /// Feeds events to the relay until nothing is left to do. Carrying out an
    /// action can produce another event, such as a budget that turned out
    /// used up.
    fn feed(&mut self, mut pending: VecDeque<Event>) {
        while let Some(event) = pending.pop_front() {
            let closed = match &event {
                Event::SessionClosed { session } => Some(*session),
                _ => None,
            };
            for action in self.relay.handle(event) {
                self.execute(action, &mut pending);
            }
            if let Some(session) = closed {
                self.forget_session(session);
            }
        }
    }

    fn execute(&mut self, action: Action, pending: &mut VecDeque<Event>) {
        if let Some(line) = describe(&action) {
            self.log(|| line);
        }
        match action {
            Action::SendControl { session, message } => {
                let Some(handle) = self.sessions.get(&session) else {
                    return;
                };
                // Full: the session does not take its control messages.
                // Closed: its control stream could not be written, which the
                // writer has reported or is about to.
                if handle.control.try_send(message).is_err() {
                    self.close(
                        session,
                        TerminationCode::InternalError.code(),
                        "control messages cannot be sent",
                        pending,
                    );
                }
            }
            Action::SendRequest {
                session,
                stream,
                message,
            } => self.request(session, stream, RequestCommand::Send(message), pending),
            Action::FinishRequest { session, stream } => {
                self.request(session, stream, RequestCommand::Finish, pending);
                if let Some(handle) = self.sessions.get_mut(&session) {
                    handle.requests.remove(&stream);
                }
            }
            Action::ResetRequest {
                session,
                stream,
                code,
            } => {
                self.request(session, stream, RequestCommand::Reset(code), pending);
                if let Some(handle) = self.sessions.get_mut(&session) {
                    handle.requests.remove(&stream);
                    handle.stops.stop(stream.0, code);
                }
            }
            Action::OpenDataStream {
                session,
                out_stream,
                header,
            } => self.open_data_stream(session, out_stream, header, pending),
            Action::SendObject { out_stream, object } => {
                let full = self
                    .out_streams
                    .get(&out_stream)
                    .is_some_and(|(_, writer)| writer.send(object).is_err());
                if full {
                    pending.push_back(Event::SendQueueFull { out_stream });
                }
            }
            // The writer stays in the table until it reports that it is done,
            // so that it can still be reset.
            Action::FinishDataStream { out_stream } => {
                if let Some((_, writer)) = self.out_streams.get(&out_stream) {
                    writer.finish();
                }
            }
            Action::ResetDataStream { out_stream, code } => {
                if let Some((_, writer)) = self.out_streams.remove(&out_stream) {
                    writer.reset(code);
                }
            }
            // Both stop a stream of the peer: its readers go by one numbering.
            Action::StopDataStream {
                session,
                stream,
                code,
            }
            | Action::StopRequest {
                session,
                stream,
                code,
            } => {
                if let Some(handle) = self.sessions.get(&session) {
                    handle.stops.stop(stream.0, code);
                }
            }
            Action::StartDrainTimer { .. }
            | Action::StartEarlyTimer { .. }
            | Action::StartRequestTimer { .. } => self.start_timer(&action),
            Action::CloseSession {
                session,
                code,
                reason,
            } => self.close(session, code, &reason, pending),
        }
    }

    /// Starts the writer of a subgroup stream towards a subscriber.
    fn open_data_stream(
        &mut self,
        session: SessionKey,
        out_stream: OutStreamKey,
        header: SubgroupHeader,
        pending: &mut VecDeque<Event>,
    ) {
        let Some(handle) = self.sessions.get(&session) else {
            return;
        };
        let events = self.events.clone();
        let report = move |event| {
            let events = events.clone();
            async move {
                let event = match event {
                    OutgoingEvent::Opened => Event::OutStreamOpened { out_stream },
                    OutgoingEvent::Closed => Event::OutStreamClosed { out_stream },
                };
                let _ = events.send(Envelope::Core(event)).await;
            }
        };
        let writer = OutgoingSubgroup::spawn(
            Arc::clone(&handle.connection),
            header,
            &handle.budget,
            report,
            self.config.streams.trace.clone(),
            format!("session#{} out#{}", session.0, out_stream.0),
        );
        if let Ok(writer) = writer {
            self.out_streams.insert(out_stream, (session, writer));
        } else {
            // Too many streams of this subscriber are not written out. No
            // writer was started, so the hub says what one would have said.
            pending.push_back(Event::SendQueueFull { out_stream });
            pending.push_back(Event::OutStreamClosed { out_stream });
        }
    }

    /// Starts the timer an action asks for.
    fn start_timer(&self, action: &Action) {
        let (wait, event) = match *action {
            Action::StartDrainTimer {
                session,
                track_alias,
            } => (
                self.config.drain_timeout,
                Event::DrainTimeout {
                    session,
                    track_alias,
                },
            ),
            Action::StartEarlyTimer { session, stream } => (
                self.config.early_stream_timeout,
                Event::EarlyStreamTimeout { session, stream },
            ),
            Action::StartRequestTimer { session, stream } => (
                self.config.request_close_timeout,
                Event::RequestTimeout { session, stream },
            ),
            _ => return,
        };
        self.after(wait, event);
    }

    /// Reports `event` to the hub after `wait`.
    fn after(&self, wait: Duration, event: Event) {
        let events = self.events.clone();
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = events.send(Envelope::Core(event)).await;
        });
    }

    /// Queues a command for the writer of a request stream. A stream whose
    /// queue is full belongs to a session that does not take its messages, and
    /// the session is closed.
    fn request(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        command: RequestCommand,
        pending: &mut VecDeque<Event>,
    ) {
        let Some(writer) = self
            .sessions
            .get(&session)
            .and_then(|handle| handle.requests.get(&stream))
        else {
            return;
        };
        // Closed means the writer ended because the peer stopped the stream,
        // which it reports itself.
        if let Err(mpsc::error::TrySendError::Full(_)) = writer.try_send(command) {
            self.close(
                session,
                TerminationCode::InternalError.code(),
                "too many messages are waiting to be sent",
                pending,
            );
        }
    }

    /// Closes a session and tells the relay it is gone.
    fn close(
        &mut self,
        session: SessionKey,
        code: u32,
        reason: &str,
        pending: &mut VecDeque<Event>,
    ) {
        let Some(handle) = self.sessions.get(&session) else {
            return;
        };
        self.log(|| {
            format!(
                "CLOSE_SESSION session={} code={code:#x} reason={reason:?}",
                session.0
            )
        });
        // WHY here and not through a writer of the session: the session is
        // forgotten right after, which stops its writers. A close left in a
        // queue would be lost, and the peer would see the connection go away
        // without the code.
        handle.connection.close(code, reason.as_bytes());
        pending.push_back(Event::SessionClosed { session });
    }

    fn forget_session(&mut self, session: SessionKey) {
        if self.sessions.remove(&session).is_some() {
            self.log(|| format!("SESSION_CLOSED session={}", session.0));
        }
        // Every writer of the session, also one that was finished and has not
        // written everything out.
        self.out_streams.retain(|_, (owner, writer)| {
            if *owner == session {
                writer.reset(stream_reset_code::SESSION_CLOSED);
            }
            *owner != session
        });
    }
}

fn uni_envelope<C: Connection>(session: SessionKey, event: UniEvent) -> Envelope<C> {
    Envelope::Core(match event {
        UniEvent::Control(message) => Event::ControlMessage { session, message },
        UniEvent::SubgroupOpened { stream, header } => Event::DataStreamOpened {
            session,
            stream: StreamKey(stream),
            header,
        },
        UniEvent::Object { stream, object, .. } => Event::Object {
            session,
            stream: StreamKey(stream),
            object,
        },
        UniEvent::TrackMalformed { stream, .. } => Event::TrackMalformed {
            session,
            stream: StreamKey(stream),
        },
        UniEvent::SubgroupClosed { stream, end, .. } => Event::DataStreamClosed {
            session,
            stream: StreamKey(stream),
            end: match end {
                StreamEnd::Fin => xmoq_relay_core::StreamEnd::Fin,
                StreamEnd::Aborted(code) => xmoq_relay_core::StreamEnd::Aborted(code),
            },
        },
        UniEvent::Failed(error) => Event::SessionFailed { session, error },
        UniEvent::ReaderPanicked => return reader_panicked(session),
    })
}

fn request_envelope<C: Connection>(
    session: SessionKey,
    event: PeerRequestStreamEvent<C::Send>,
) -> Envelope<C> {
    match event {
        PeerRequestStreamEvent::Opened {
            stream,
            send,
            first,
        } => Envelope::RequestOpened {
            session,
            stream: StreamKey(stream),
            send,
            first,
        },
        PeerRequestStreamEvent::Message { stream, message } => {
            Envelope::Core(Event::RequestMessage {
                session,
                stream: StreamKey(stream),
                message,
            })
        }
        PeerRequestStreamEvent::Closed { stream } => Envelope::Core(Event::RequestClosed {
            session,
            stream: StreamKey(stream),
        }),
        PeerRequestStreamEvent::Failed(error) => {
            Envelope::Core(Event::SessionFailed { session, error })
        }
        PeerRequestStreamEvent::ReaderPanicked => reader_panicked(session),
    }
}

fn reader_panicked<C: Connection>(session: SessionKey) -> Envelope<C> {
    Envelope::Broken {
        session,
        code: TerminationCode::InternalError,
        reason: "a stream reader failed",
    }
}

/// Owns the readers of one session and reports the session closed.
///
/// WHY not at once when the connection goes: what had arrived before can
/// still be read (see the contract of `xmoq_moqt_io::RecvStream::read`), and
/// a publisher that sends its last Object and `PUBLISH_DONE` and then closes
/// is exactly that case. The readers put what they read in the same queue
/// this report goes to, so waiting for them keeps the order.
/// WHY a limit on the wait: a reader that is held back by a full queue must
/// not keep a dead session.
async fn watch_session<C: Connection>(
    session: SessionKey,
    connection: Arc<C>,
    mut readers: [IncomingStreams; 2],
    closed_read_timeout: Duration,
    events: mpsc::Sender<Envelope<C>>,
) {
    connection.closed().await;
    let read = async {
        for reader in &mut readers {
            reader.done().await;
        }
    };
    let _ = tokio::time::timeout(closed_read_timeout, read).await;
    let _ = events
        .send(Envelope::Core(Event::SessionClosed { session }))
        .await;
}

/// Writes the relay's control stream of one session.
async fn write_control<C: Connection>(
    session: SessionKey,
    connection: Arc<C>,
    mut messages: mpsc::Receiver<ControlMessage>,
    trace: Trace,
    events: mpsc::Sender<Envelope<C>>,
) {
    // §3.3: the control stream stays open for the whole session, so it lives
    // as long as this task, which the session's handle ends.
    let Ok(mut control) = connection.open_uni().await else {
        return;
    };
    loop {
        let message = tokio::select! {
            message = messages.recv() => message,
            stopped = control.stopped() => {
                if let WriteError::Stopped(_) = stopped {
                    // §3.3: closing a control stream is a PROTOCOL_VIOLATION,
                    // and a STOP_SENDING closes this one.
                    let broken = Envelope::Broken {
                        session,
                        code: TerminationCode::ProtocolViolation,
                        reason: "the peer stopped the relay's control stream",
                    };
                    let _ = events.send(broken).await;
                }
                return;
            }
        };
        let Some(message) = message else { return };
        if let Err(error) = send_message(&mut control, &message, &trace, "control").await {
            let broken = Envelope::Broken {
                session,
                code: TerminationCode::InternalError,
                reason: "the control stream could not be written",
            };
            // A lost connection is reported by the watcher of the session.
            if !matches!(error, xmoq_moqt_io::SendError::Write(WriteError::Lost(_))) {
                let _ = events.send(broken).await;
            }
            return;
        }
    }
}

/// Writes the relay's half of one request stream, and watches for the peer
/// stopping it.
async fn write_request<C: Connection>(
    session: SessionKey,
    stream: StreamKey,
    mut send: C::Send,
    mut commands: mpsc::Receiver<RequestCommand>,
    trace: Trace,
    events: mpsc::Sender<Envelope<C>>,
) {
    let label = format!("bi#{}", stream.0);
    loop {
        let command = tokio::select! {
            command = commands.recv() => command,
            stopped = send.stopped() => {
                if let WriteError::Stopped(_) = stopped {
                    // §3.3.2, §5.1: STOP_SENDING on a request stream cancels
                    // the request. The relay's half is reset in turn.
                    send.reset(stream_reset_code::CANCELLED);
                    let closed = Event::RequestClosed { session, stream };
                    let _ = events.send(Envelope::Core(closed)).await;
                }
                return;
            }
        };
        match command {
            Some(RequestCommand::Send(message)) => {
                // An error means the peer stopped the stream or is gone, which
                // the other arm of the select reports.
                let _ = send_message(&mut send, &message, &trace, &label).await;
            }
            // `None`: the hub let go of the stream because the peer closed its
            // direction; the relay's is closed in order.
            Some(RequestCommand::Finish) | None => {
                send.finish();
                return;
            }
            Some(RequestCommand::Reset(code)) => {
                send.reset(code);
                return;
            }
        }
    }
}

/// One line for the log about an action, for the actions that are rare enough
/// to log. An Object forwarded to a subscriber is not.
fn describe(action: &Action) -> Option<String> {
    Some(match action {
        Action::SendControl { session, message } => {
            format!("SEND session={} control {}", session.0, name(message))
        }
        Action::SendRequest {
            session,
            stream,
            message,
        } => format!(
            "SEND session={} stream={} {}",
            session.0,
            stream.0,
            name(message)
        ),
        Action::FinishRequest { session, stream } => {
            format!("FINISH_REQUEST session={} stream={}", session.0, stream.0)
        }
        Action::ResetRequest {
            session,
            stream,
            code,
        } => format!(
            "RESET_REQUEST session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        Action::OpenDataStream {
            session,
            out_stream,
            header,
        } => format!(
            "OPEN_DATA session={} out={} track_alias={} group={}",
            session.0, out_stream.0, header.track_alias, header.group_id
        ),
        Action::FinishDataStream { out_stream } => format!("FINISH_DATA out={}", out_stream.0),
        Action::ResetDataStream { out_stream, code } => {
            format!("RESET_DATA out={} code={code:#x}", out_stream.0)
        }
        Action::StopDataStream {
            session,
            stream,
            code,
        } => format!(
            "STOP_DATA session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        Action::StartDrainTimer {
            session,
            track_alias,
        } => format!(
            "DRAIN_TIMER session={} track_alias={track_alias}",
            session.0
        ),
        Action::StopRequest {
            session,
            stream,
            code,
        } => format!(
            "STOP_REQUEST session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        // Logged where the session is closed, whoever asked for it.
        Action::CloseSession { .. }
        | Action::SendObject { .. }
        | Action::StartEarlyTimer { .. }
        | Action::StartRequestTimer { .. } => return None,
    })
}

fn name(message: &ControlMessage) -> String {
    match message {
        ControlMessage::Setup(_) => "SETUP".to_string(),
        ControlMessage::RequestUpdate(_) => "REQUEST_UPDATE".to_string(),
        ControlMessage::Subscribe(_) => "SUBSCRIBE".to_string(),
        ControlMessage::SubscribeOk(ok) => format!("SUBSCRIBE_OK track_alias={}", ok.track_alias),
        ControlMessage::Publish(_) => "PUBLISH".to_string(),
        ControlMessage::PublishNamespace(_) => "PUBLISH_NAMESPACE".to_string(),
        ControlMessage::RequestOk(_) => "REQUEST_OK".to_string(),
        ControlMessage::RequestError(error) => {
            format!("REQUEST_ERROR error_code={:#x}", error.error_code)
        }
        ControlMessage::PublishDone(done) => format!(
            "PUBLISH_DONE status_code={:#x} stream_count={}",
            done.status_code, done.stream_count
        ),
        ControlMessage::UnsupportedRequest { message_type, .. } => {
            format!("unsupported request {message_type:#x}")
        }
    }
}
