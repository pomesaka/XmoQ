//! Connects the sessions of the relay to its one decision maker.
//!
//! Every session has readers that turn its streams into [`Event`]s. One task,
//! the hub, feeds them to the [`Relay`] in the order they arrive and carries out
//! the [`Action`]s it returns. The hub never waits for a peer and never writes
//! to anything that can block: what it hands to a session goes into a bounded
//! queue, and a queue that is full ends that session or that subscription
//! instead of holding up the others.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use xmoq_moqt::data::SubgroupHeader;
use xmoq_moqt::fetch::FetchHeader;
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::session::MAX_PEER_REQUESTS;
use xmoq_moqt::{TerminationCode, stream_reset_code};
use xmoq_moqt_io::{
    Connection, FetchBody, FetchChunk, IncomingStreams, OutgoingEvent, OutgoingSubgroup,
    PeerRequestClose, PeerRequestStreamEvent, RecvStream, RequestClose, RequestReadError,
    RequestReader, RequestStreamEvent, SendBudget, SendLimits, SendStream, StreamConfig, StreamEnd,
    StreamStops, Trace, UniEvent, WriteError, open_fetch_stream, read_request_streams,
    read_uni_streams, send_message,
};
use xmoq_relay_core::{
    Action, Event, FetchPipeEnd, OutStreamKey, OwnRequestEnd, OwnRequestKey, Relay, RequestEnd,
    Role, SessionKey, StreamKey,
};

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
/// | QUIC: datagrams it sent, which nothing reads | 1500 bytes |
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
    /// Timers one session may have running at once. A timer beyond that is
    /// not started and its event is reported at once.
    pub max_timers_per_session: usize,
    /// How long a publisher gets to answer a SUBSCRIBE the relay sent it for
    /// a track of a namespace it announced. Then the subscribers that wait
    /// are answered with `TIMEOUT` (§10.6.2).
    pub upstream_timeout: Duration,
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
            max_timers_per_session: 1024,
            upstream_timeout: Duration::from_secs(5),
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
    /// A log whose lines a test reads.
    #[cfg(test)]
    pub fn collect() -> (Self, mpsc::Receiver<String>) {
        let (lines, collected) = mpsc::channel(1024);
        let log = Self {
            lines,
            dropped: Arc::new(AtomicU64::new(0)),
        };
        (log, collected)
    }

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
    /// Tasks of request streams the relay opened, over all sessions.
    pub own_requests: usize,
    /// Fetch streams that are being passed on.
    pub fetch_pipes: usize,
    /// Timers that are running.
    pub timers: usize,
    /// What the relay holds.
    pub relay: xmoq_relay_core::Counts,
}

/// What the hub asks the task of a request stream the relay opened to do.
/// Either ends the task.
enum OwnRequestCommand {
    Finish,
    Cancel(u32),
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
    /// The tasks of the request streams the relay has open on the session.
    own_requests: HashMap<OwnRequestKey, mpsc::Sender<OwnRequestCommand>>,
    stops: StreamStops,
    /// What waits to be written to this session's subgroup streams.
    budget: Arc<SendBudget>,
    /// The writers of its control and request streams, and the task that owns
    /// its readers.
    tasks: Vec<JoinHandle<()>>,
    /// The tasks that write a fetch stream the relay made itself and are not
    /// done.
    fetch_writes: Vec<JoinHandle<()>>,
}

impl<C: Connection> Drop for SessionHandle<C> {
    fn drop(&mut self) {
        for task in self.tasks.iter().chain(&self.fetch_writes) {
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
        fetch_pipes: HashMap::new(),
        timers: Timers::default(),
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
    /// The fetch streams that are being passed on, by the subscriber's
    /// session and the request stream of its FETCH, each with the way to cut
    /// it short. Dropping that cuts it short as well.
    fetch_pipes: HashMap<(SessionKey, StreamKey), oneshot::Sender<u32>>,
    timers: Timers,
}

/// The timers the relay asked for, in one place.
///
/// WHY not a task per timer: a peer decides how many timers start (every
/// stream it opens ahead of its PUBLISH starts one), and a task cannot be
/// taken back when its session goes. Here a session's timers are counted,
/// bounded, and dropped with the session.
#[derive(Default)]
struct Timers {
    /// By the time they are due, and a number that keeps equal times apart.
    pending: BTreeMap<(Instant, u64), (SessionKey, Event)>,
    per_session: HashMap<SessionKey, usize>,
    /// Where the timer for the answer to a request of the relay is, so that
    /// it can be dropped when the answer came.
    upstream: HashMap<(SessionKey, OwnRequestKey), (Instant, u64)>,
    next: u64,
}

impl Timers {
    /// Starts a timer and returns `None`, unless the session already has
    /// `limit` of them running; then the event is handed back.
    fn start(
        &mut self,
        session: SessionKey,
        limit: usize,
        wait: Duration,
        event: Event,
    ) -> Option<Event> {
        // A new wait for the same request replaces the one that runs, and
        // takes its place in the count.
        if let Event::UpstreamTimeout { session, request } = event {
            self.stop_upstream(session, request);
        }
        let running = self.per_session.entry(session).or_default();
        if *running >= limit {
            return Some(event);
        }
        *running += 1;
        let at = (Instant::now() + wait, self.next);
        if let Event::UpstreamTimeout { session, request } = event {
            self.upstream.insert((session, request), at);
        }
        self.pending.insert(at, (session, event));
        self.next = self.next.wrapping_add(1);
        None
    }

    /// Drops the timer for the answer to a request of the relay.
    fn stop_upstream(&mut self, session: SessionKey, request: OwnRequestKey) {
        let Some(at) = self.upstream.remove(&(session, request)) else {
            return;
        };
        if self.pending.remove(&at).is_some()
            && let Some(running) = self.per_session.get_mut(&session)
        {
            *running = running.saturating_sub(1);
        }
    }

    /// Completes when the next timer is due; never, when none is running.
    async fn due(&self) {
        match self.pending.first_key_value() {
            Some(((at, _), _)) => tokio::time::sleep_until(*at).await,
            None => std::future::pending().await,
        }
    }

    fn take_due(&mut self) -> VecDeque<Event> {
        let now = Instant::now();
        let mut events = VecDeque::new();
        while let Some(entry) = self.pending.first_entry() {
            if entry.key().0 > now {
                break;
            }
            let (session, event) = entry.remove();
            if let Some(running) = self.per_session.get_mut(&session) {
                *running = running.saturating_sub(1);
            }
            if let Event::UpstreamTimeout { session, request } = event {
                self.upstream.remove(&(session, request));
            }
            events.push_back(event);
        }
        events
    }

    fn forget_session(&mut self, session: SessionKey) {
        self.per_session.remove(&session);
        self.pending.retain(|_, (owner, _)| *owner != session);
        self.upstream.retain(|(owner, _), _| *owner != session);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pending.len()
    }
}

impl<C: Connection> Hub<C> {
    async fn run(mut self, mut inbox: mpsc::Receiver<Envelope<C>>) {
        loop {
            let envelope = tokio::select! {
                envelope = inbox.recv() => envelope,
                () = self.timers.due() => {
                    let events = self.timers.take_due();
                    self.feed(events);
                    continue;
                }
            };
            let Some(envelope) = envelope else { return };
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
                        // The peer ended the stream, or stopped the relay's
                        // direction of it; either way the peer's direction is
                        // over (§3.3.2). What becomes of the relay's is for the
                        // relay to say, with FinishRequest or ResetRequest.
                        Event::RequestClosed {
                            session,
                            stream,
                            how: RequestEnd::Aborted,
                        } => {
                            if let Some(handle) = self.sessions.get(session) {
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
                        own_requests: self
                            .sessions
                            .values()
                            .map(|handle| handle.own_requests.len())
                            .sum(),
                        fetch_pipes: self.fetch_pipes.len(),
                        timers: self.timers.len(),
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
            {
                let log = self.config.log.clone();
                move |event| request_envelope(session, event, log.as_ref())
            },
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
            self.config.log.clone(),
            self.events.clone(),
        ));
        self.sessions.insert(
            session,
            SessionHandle {
                connection,
                control,
                requests: HashMap::new(),
                own_requests: HashMap::new(),
                stops,
                budget: SendBudget::new(self.config.send),
                tasks: vec![writer, watcher],
                fetch_writes: Vec::new(),
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
            self.config.log.clone(),
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
            // What a publisher answered to a request of the relay: the one
            // thing the relay acts on that none of its own lines shows.
            if let Event::OwnRequestMessage {
                session,
                request,
                message,
            } = &event
            {
                self.log(|| {
                    format!(
                        "RECV session={} own={} {}",
                        session.0,
                        request.0,
                        name(message)
                    )
                });
            }
            if let Event::FetchPipeEnded {
                session,
                stream,
                how,
            } = &event
            {
                // The task is done; nothing is left to cut short.
                self.fetch_pipes.remove(&(*session, *stream));
                self.log(|| {
                    format!(
                        "FETCH_PIPE_ENDED session={} stream={} {}",
                        session.0,
                        stream.0,
                        pipe_end(*how)
                    )
                });
            }
            for action in self.relay.handle(event) {
                self.execute(action, &mut pending);
            }
            if let Some(session) = closed {
                self.forget_session(session);
            }
        }
    }

    fn log_action(&self, action: &Action) {
        if let Some(line) = describe(action) {
            self.log(|| line);
        }
    }

    fn execute(&mut self, action: Action, pending: &mut VecDeque<Event>) {
        self.log_action(&action);
        let Some(action) = self.execute_upstream(action, pending) else {
            return;
        };
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
            | Action::StartRequestTimer { .. }
            | Action::StartUpstreamTimer { .. }
            | Action::StopUpstreamTimer { .. } => self.set_timer(&action, pending),
            Action::CloseSession {
                session,
                code,
                reason,
            } => self.close(session, code, &reason, pending),
            // Taken by `execute_upstream` above.
            Action::OpenRequest { .. }
            | Action::FinishOwnRequest { .. }
            | Action::CancelOwnRequest { .. }
            | Action::PipeFetch { .. }
            | Action::AbortFetchPipe { .. }
            | Action::WriteFetchStream { .. } => {}
        }
    }

    /// Carries out the actions that are about a request the relay opened, or
    /// about a fetch stream. Returns every other action.
    fn execute_upstream(
        &mut self,
        action: Action,
        pending: &mut VecDeque<Event>,
    ) -> Option<Action> {
        match action {
            Action::OpenRequest {
                session,
                request,
                message,
            } => self.open_request(session, request, message),
            Action::FinishOwnRequest { session, request } => {
                self.own_request(session, request, OwnRequestCommand::Finish);
            }
            Action::CancelOwnRequest {
                session,
                request,
                code,
            } => self.own_request(session, request, OwnRequestCommand::Cancel(code)),
            Action::PipeFetch {
                from,
                from_stream,
                session,
                stream,
                request_id,
                upstream_wait_ms,
                downstream_wait_ms,
            } => {
                let waits = PipeWaits {
                    upstream: Duration::from_millis(upstream_wait_ms),
                    downstream: Duration::from_millis(downstream_wait_ms),
                };
                let header = FetchHeader { request_id };
                self.pipe_fetch(
                    (from, from_stream),
                    (session, stream),
                    header,
                    waits,
                    pending,
                );
            }
            Action::AbortFetchPipe {
                session,
                stream,
                code,
            } => {
                if let Some(abort) = self.fetch_pipes.remove(&(session, stream)) {
                    // The task may have just ended; then nobody needs this.
                    let _ = abort.send(code);
                }
            }
            Action::WriteFetchStream {
                session,
                stream,
                request_id,
                body,
                wait_ms,
            } => self.write_fetch_stream((session, stream), request_id, body, wait_ms),
            other => return Some(other),
        }
        None
    }

    /// Writes a whole fetch stream that the relay made itself, and tells the
    /// relay when that is over.
    fn write_fetch_stream(
        &mut self,
        (session, stream): (SessionKey, StreamKey),
        request_id: u64,
        body: Vec<u8>,
        wait_ms: u64,
    ) {
        let written = Event::FetchStreamWritten { session, stream };
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        // The relay lets a session have only so many of these at once, so
        // the list stays short. The tasks go with the session.
        handle.fetch_writes.retain(|task| !task.is_finished());
        let connection = Arc::clone(&handle.connection);
        let trace = self.config.streams.trace.clone();
        let label = format!("session#{} fetch#{request_id}", session.0);
        let patience = Duration::from_millis(wait_ms);
        let events = self.events.clone();
        handle.fetch_writes.push(tokio::spawn(async move {
            let writing = tokio::spawn(async move {
                let header = FetchHeader { request_id };
                let opened = tokio::time::timeout(
                    patience,
                    open_fetch_stream(&*connection, header, &trace, &label),
                );
                // Not opened in time, or not at all: the writer that was
                // dropped with the wait has reset what it had opened.
                let Ok(Ok(mut writer)) = opened.await else {
                    return;
                };
                match tokio::time::timeout(patience, writer.write(&body)).await {
                    Ok(Ok(())) => writer.finish(),
                    // Stopped by the subscriber, or the connection is gone.
                    Ok(Err(_)) => writer.reset(stream_reset_code::CANCELLED),
                    // As for a fetch stream that is passed on and not taken.
                    Err(_) => writer.reset(stream_reset_code::TOO_FAR_BEHIND),
                }
            });
            // Told whatever became of the write, a panic included.
            let _ = writing.await;
            let _ = events.send(Envelope::Core(written)).await;
        }));
    }

    /// Starts passing a publisher's fetch stream on to a subscriber.
    fn pipe_fetch(
        &mut self,
        (from, from_stream): (SessionKey, StreamKey),
        subscriber: (SessionKey, StreamKey),
        header: FetchHeader,
        waits: PipeWaits,
        pending: &mut VecDeque<Event>,
    ) {
        let (session, stream) = subscriber;
        let ended = move |how| Event::FetchPipeEnded {
            session,
            stream,
            how,
        };
        let Some(publisher) = self.sessions.get(&from) else {
            pending.push_back(ended(FetchPipeEnd::UpstreamLost));
            return;
        };
        // Taken at once, so that the publisher's stream frees its slot among
        // the streams read of that session.
        let Some(body) = publisher.stops.take_fetch_body(from_stream.0) else {
            pending.push_back(ended(FetchPipeEnd::UpstreamLost));
            return;
        };
        // The body, dropped here, stops the publisher's stream.
        let Some(handle) = self.sessions.get(&session) else {
            pending.push_back(ended(FetchPipeEnd::DownstreamLost));
            return;
        };
        let (abort, aborted) = oneshot::channel();
        self.fetch_pipes.insert(subscriber, abort);
        let pipe = FetchPipe {
            connection: Arc::clone(&handle.connection),
            header,
            body,
            waits,
            trace: self.config.streams.trace.clone(),
            label: format!("session#{} fetch#{}", session.0, header.request_id),
        };
        let events = self.events.clone();
        tokio::spawn(async move {
            // WHY a task of its own for the passing on: the relay is told of
            // its end here whatever becomes of it, a panic included, or the
            // FETCH would stay in the relay for as long as its sessions do.
            // Both streams go with that task: the writer resets the
            // subscriber's, and the body, dropped, stops the publisher's.
            let how = match tokio::spawn(pipe.run(aborted)).await {
                Ok(how) => how,
                Err(_) => Some(FetchPipeEnd::UpstreamLost),
            };
            if let Some(how) = how {
                let _ = events.send(Envelope::Core(ended(how))).await;
            }
        });
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
        let log = self.config.log.clone();
        let report = move |event| {
            let events = events.clone();
            let log = log.clone();
            async move {
                let event = match event {
                    OutgoingEvent::Opened => Event::OutStreamOpened { out_stream },
                    OutgoingEvent::Closed => Event::OutStreamClosed { out_stream },
                    // For the log only: the writer resets the stream and
                    // reports it closed.
                    OutgoingEvent::Stopped(code) => {
                        if let Some(log) = &log {
                            log.print(format!(
                                "STOP_SENDING_RECEIVED session={} out={} {}",
                                session.0,
                                out_stream.0,
                                stop_code(code)
                            ));
                        }
                        return;
                    }
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

    /// Starts or stops the timer an action asks for.
    fn set_timer(&mut self, action: &Action, pending: &mut VecDeque<Event>) {
        let (session, wait, event) = match *action {
            Action::StopUpstreamTimer { session, request } => {
                self.timers.stop_upstream(session, request);
                return;
            }
            Action::StartDrainTimer {
                session,
                track_alias,
                wait,
            } => (
                session,
                self.config.drain_timeout,
                Event::DrainTimeout {
                    session,
                    track_alias,
                    wait,
                },
            ),
            Action::StartEarlyTimer { session, stream } => (
                session,
                self.config.early_stream_timeout,
                Event::EarlyStreamTimeout { session, stream },
            ),
            Action::StartRequestTimer { session, stream } => (
                session,
                self.config.request_close_timeout,
                Event::RequestTimeout { session, stream },
            ),
            Action::StartUpstreamTimer { session, request } => (
                session,
                self.config.upstream_timeout,
                Event::UpstreamTimeout { session, request },
            ),
            _ => return,
        };
        let limit = self.config.max_timers_per_session;
        if let Some(event) = self.timers.start(session, limit, wait, event) {
            // WHY at once: see `Action::StartUpstreamTimer`. Whatever the
            // timer guarded is let go of now instead of never.
            self.log(|| format!("TIMER_NOT_STARTED session={} too many timers", session.0));
            pending.push_back(event);
        }
    }

    /// Starts the task of a request the relay opens on a session.
    fn open_request(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        message: ControlMessage,
    ) {
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        // Room for the one command that ends the task.
        let (commands, command_inbox) = mpsc::channel(1);
        let task = tokio::spawn(run_own_request(
            session,
            request,
            Arc::clone(&handle.connection),
            message,
            command_inbox,
            self.config.streams.trace.clone(),
            self.events.clone(),
        ));
        handle.own_requests.insert(request, commands);
        handle.tasks.retain(|task| !task.is_finished());
        handle.tasks.push(task);
    }

    /// Ends the task of a request the relay opened, and the timer for its
    /// answer with it.
    fn own_request(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        command: OwnRequestCommand,
    ) {
        self.timers.stop_upstream(session, request);
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        if let Some(commands) = handle.own_requests.remove(&request) {
            // Closed: the task ended by itself, with the stream.
            let _ = commands.try_send(command);
        }
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
        self.timers.forget_session(session);
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
                StreamEnd::Lost => xmoq_relay_core::StreamEnd::Lost,
            },
        },
        UniEvent::FetchOpened { stream, header } => Event::FetchStreamOpened {
            session,
            stream: StreamKey(stream),
            request_id: header.request_id,
        },
        UniEvent::Failed(error) => Event::SessionFailed { session, error },
        UniEvent::ReaderPanicked => return reader_panicked(session),
    })
}

fn request_envelope<C: Connection>(
    session: SessionKey,
    event: PeerRequestStreamEvent<C::Send>,
    log: Option<&Log>,
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
        PeerRequestStreamEvent::Closed { stream, how } => {
            // WHY a line for the reset as well as for STOP_SENDING: a peer
            // that cancels sends both (§3.3.2), and whichever the relay sees
            // first ends the request; the other is then not looked at.
            if let (PeerRequestClose::Reset(code), Some(log)) = (how, log) {
                log.print(format!(
                    "RESET_RECEIVED session={} stream={stream} {}",
                    session.0,
                    stop_code(code)
                ));
            }
            Envelope::Core(Event::RequestClosed {
                session,
                stream: StreamKey(stream),
                how: if how.is_fin() {
                    RequestEnd::Fin
                } else {
                    RequestEnd::Aborted
                },
            })
        }
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
    log: Option<Log>,
    events: mpsc::Sender<Envelope<C>>,
) {
    let end = connection.closed().await;
    if let Some(log) = &log {
        log.print(format!("CONNECTION_ENDED session={} {end:?}", session.0));
    }
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

/// How long after a `STOP_SENDING` on the relay's control stream the peer's
/// connection may end for the stop to count as part of its leaving.
/// Provisional.
const LEAVING_GRACE: Duration = Duration::from_millis(200);

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
                    // WHY wait and see: an endpoint that goes away lets go of
                    // its streams, which QUIC stacks turn into STOP_SENDING,
                    // and the close of its connection can arrive a moment
                    // later. That is a peer leaving, and the session then
                    // ends by itself, with what had arrived still read.
                    let leaving = tokio::time::timeout(LEAVING_GRACE, connection.closed());
                    if leaving.await.is_ok() {
                        return;
                    }
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
    log: Option<Log>,
    events: mpsc::Sender<Envelope<C>>,
) {
    let label = format!("bi#{}", stream.0);
    loop {
        let command = tokio::select! {
            command = commands.recv() => command,
            stopped = send.stopped() => {
                if let WriteError::Stopped(code) = stopped {
                    if let Some(log) = &log {
                        log.print(format!(
                            "STOP_SENDING_RECEIVED session={} stream={} {}",
                            session.0,
                            stream.0,
                            stop_code(code)
                        ));
                    }
                    // §3.3.2, §5.1: STOP_SENDING on a request stream cancels
                    // the request. The relay's half is reset in turn.
                    send.reset(stream_reset_code::CANCELLED);
                    let closed = Event::RequestClosed {
                        session,
                        stream,
                        how: RequestEnd::Aborted,
                    };
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
            // `None`: the session is going.
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

/// Runs one request the relay opened: opens the stream, sends the request,
/// reports what the peer answers, and ends the stream the way the hub says.
async fn run_own_request<C: Connection>(
    session: SessionKey,
    request: OwnRequestKey,
    connection: Arc<C>,
    message: ControlMessage,
    mut commands: mpsc::Receiver<OwnRequestCommand>,
    trace: Trace,
    events: mpsc::Sender<Envelope<C>>,
) {
    let label = format!("own#{}", request.0);
    let ended = |how| {
        Envelope::Core(Event::OwnRequestClosed {
            session,
            request,
            how,
        })
    };
    // Opening waits at the peer's stream limit; a cancel must get through.
    let opened = tokio::select! {
        opened = connection.open_bi() => opened,
        _ = commands.recv() => return,
    };
    let Ok((mut send, recv)) = opened else {
        let _ = events.send(ended(OwnRequestEnd::Unsent)).await;
        return;
    };
    // Writing waits while the peer gives no flow control credit; a cancel
    // must get through here as well.
    let written = tokio::select! {
        written = send_message(&mut send, &message, &trace, &label) => Some(written.is_ok()),
        _ = commands.recv() => None,
    };
    if written != Some(true) {
        send.reset(stream_reset_code::CANCELLED);
        recv.stop(stream_reset_code::CANCELLED);
        if written == Some(false) {
            let _ = events.send(ended(OwnRequestEnd::Unsent)).await;
        }
        return;
    }
    let mut reader = RequestReader::new(recv, &trace, &label);
    // Each direction is reported once; after that only the other one, and the
    // hub's word, are waited for. What an ending means is the relay's to say.
    let (mut reading, mut watching) = (true, true);
    let command = loop {
        tokio::select! {
            command = commands.recv() => break command,
            event = reader.next_event(), if reading => match event {
                Ok(RequestStreamEvent::Message(message)) => {
                    let event = Event::OwnRequestMessage { session, request, message };
                    if events.send(Envelope::Core(event)).await.is_err() {
                        break None;
                    }
                }
                Ok(RequestStreamEvent::Closed(how)) => {
                    reading = false;
                    let how = match how {
                        RequestClose::Fin => OwnRequestEnd::Fin,
                        RequestClose::Reset(_) | RequestClose::Stopped(_) => OwnRequestEnd::Reset,
                    };
                    let _ = events.send(ended(how)).await;
                }
                Err(RequestReadError::Peer(error)) => {
                    let failed = Event::SessionFailed { session, error };
                    let _ = events.send(Envelope::Core(failed)).await;
                    break None;
                }
                // A lost connection is reported by the watcher of the session.
                Err(RequestReadError::Lost) => break None,
            },
            stopped = send.stopped(), if watching => {
                watching = false;
                if let WriteError::Stopped(_) = stopped {
                    let _ = events.send(ended(OwnRequestEnd::Stopped)).await;
                }
            }
        }
    };
    if let Some(OwnRequestCommand::Finish) = command {
        send.finish();
        reader.stop(stream_reset_code::CANCELLED);
        return;
    }
    // `None`: the session is going; the stream is not left half open.
    let code = match command {
        Some(OwnRequestCommand::Cancel(code)) => code,
        _ => stream_reset_code::CANCELLED,
    };
    send.reset(code);
    reader.stop(code);
}

/// How the passing on of a fetch stream ended, for the log. The three waits
/// of a relayed FETCH show apart: the answer that did not come is a
/// `REQUEST_ERROR` with `TIMEOUT` and no pipe, a publisher that sent nothing
/// is `upstream_stalled`, a subscriber that took nothing `downstream_stalled`.
fn pipe_end(how: FetchPipeEnd) -> String {
    match how {
        FetchPipeEnd::Finished => "finished".to_string(),
        FetchPipeEnd::UpstreamAborted(code) => format!("upstream_reset {}", stop_code(code)),
        FetchPipeEnd::UpstreamLost => "upstream_lost".to_string(),
        FetchPipeEnd::UpstreamStalled => "upstream_stalled".to_string(),
        FetchPipeEnd::DownstreamStopped(code) => {
            format!("downstream_stopped {}", stop_code(code))
        }
        FetchPipeEnd::DownstreamStalled => "downstream_stalled".to_string(),
        FetchPipeEnd::DownstreamLost => "downstream_lost".to_string(),
    }
}

/// Bytes of a fetch stream written to the subscriber at a time. A subscriber
/// that does not take this much within its wait takes nothing, as far as the
/// relay goes.
const PIPE_PIECE: usize = 1024;

/// How long the two ends of a fetch stream that is passed on may keep the
/// relay waiting.
#[derive(Clone, Copy)]
struct PipeWaits {
    /// For bytes of the publisher, while the subscriber can take more.
    upstream: Duration,
    /// For the subscriber to take what was read.
    downstream: Duration,
}

/// One fetch stream of a publisher on its way to a subscriber.
///
/// WHY a task that reads and writes in turn, where a subgroup stream goes
/// through the relay's queues: a FETCH asks for what exists already, so the
/// publisher can send as fast as its link allows and the subscriber sets the
/// pace. A queue would be full at once, and a subscriber that is merely
/// slower than the publisher would lose its FETCH. Read only when the last
/// read is written, the publisher is held back by flow control instead.
/// What the relay holds of the stream then is the read in hand here and the
/// five the reader of the publisher's stream holds (`FetchBody`), 16 KiB
/// each: 96 KiB, next to the windows of the two connections.
///
/// The waits start anew with every read and every write: a stream that
/// goes on, however slowly, is not cut, and a FETCH has no time limit as a
/// whole.
struct FetchPipe<C: Connection> {
    connection: Arc<C>,
    header: FetchHeader,
    body: FetchBody,
    waits: PipeWaits,
    trace: Trace,
    label: String,
}

impl<C: Connection> FetchPipe<C> {
    /// Passes the stream on to its end. `None` when it was cut short from
    /// outside; then the one who cut it knows.
    async fn run(mut self, mut aborted: oneshot::Receiver<u32>) -> Option<FetchPipeEnd> {
        // An error means the hub dropped its end: the subscriber's session
        // is gone.
        let cut = |code: Result<u32, oneshot::error::RecvError>| {
            code.unwrap_or(stream_reset_code::SESSION_CLOSED)
        };
        let opened = tokio::select! {
            biased;
            _ = &mut aborted => return None,
            opened = tokio::time::timeout(
                self.waits.downstream,
                open_fetch_stream(&*self.connection, self.header, &self.trace, &self.label),
            ) => opened,
        };
        let mut send = match opened {
            Ok(Ok(send)) => send,
            Ok(Err(WriteError::Stopped(code))) => {
                return Some(FetchPipeEnd::DownstreamStopped(code));
            }
            Ok(Err(WriteError::Lost(_))) => return Some(FetchPipeEnd::DownstreamLost),
            // The subscriber gives the relay no stream to write on.
            Err(_) => return Some(FetchPipeEnd::DownstreamStalled),
        };
        loop {
            let chunk = tokio::select! {
                biased;
                code = &mut aborted => {
                    send.reset(cut(code));
                    return None;
                }
                // §5.2: the subscriber "MAY send STOP_SENDING for the data
                // stream" of a FETCH it cancels.
                stopped = send.stopped() => {
                    return Some(match stopped {
                        WriteError::Stopped(code) => {
                            // §11.4.3: a reset answers a STOP_SENDING.
                            send.reset(stream_reset_code::CANCELLED);
                            FetchPipeEnd::DownstreamStopped(code)
                        }
                        WriteError::Lost(_) => FetchPipeEnd::DownstreamLost,
                    });
                }
                chunk = tokio::time::timeout(self.waits.upstream, self.body.next()) => chunk,
            };
            let bytes = match chunk {
                Ok(FetchChunk::Bytes(bytes)) => bytes,
                Ok(FetchChunk::End(StreamEnd::Fin)) => {
                    send.finish();
                    return Some(FetchPipeEnd::Finished);
                }
                Ok(FetchChunk::End(StreamEnd::Aborted(code))) => {
                    send.reset(xmoq_relay_core::fetch_reset_passed_on(code));
                    return Some(FetchPipeEnd::UpstreamAborted(code));
                }
                // The publisher is gone, or its readers are: what came next
                // will not be known (§3.3.3, UNKNOWN_OBJECT_STATUS).
                Ok(FetchChunk::End(StreamEnd::Lost) | FetchChunk::Stopped) => {
                    send.reset(stream_reset_code::UNKNOWN_OBJECT_STATUS);
                    return Some(FetchPipeEnd::UpstreamLost);
                }
                Err(_) => {
                    send.reset(stream_reset_code::UNKNOWN_OBJECT_STATUS);
                    return Some(FetchPipeEnd::UpstreamStalled);
                }
            };
            // WHY in pieces, each with the wait anew: the wait is for a
            // subscriber that takes nothing, and a write of a whole read
            // that does not end in time says only that it did not take all
            // of it. A subscriber on a slow link takes a piece now and then.
            for piece in bytes.chunks(PIPE_PIECE) {
                let written = tokio::select! {
                    biased;
                    code = &mut aborted => {
                        send.reset(cut(code));
                        return None;
                    }
                    written = tokio::time::timeout(self.waits.downstream, send.write(piece)) => written,
                };
                match written {
                    Ok(Ok(())) => {}
                    Ok(Err(WriteError::Stopped(code))) => {
                        send.reset(stream_reset_code::CANCELLED);
                        return Some(FetchPipeEnd::DownstreamStopped(code));
                    }
                    Ok(Err(WriteError::Lost(_))) => return Some(FetchPipeEnd::DownstreamLost),
                    // WHY TOO_FAR_BEHIND, which §3.3.3 defines for a
                    // subscription: the draft has no code for a FETCH whose
                    // subscriber does not take it, and this one says what
                    // happened, that the receiver went over what the sender
                    // would hold for it.
                    Err(_) => {
                        send.reset(stream_reset_code::TOO_FAR_BEHIND);
                        return Some(FetchPipeEnd::DownstreamStalled);
                    }
                }
            }
        }
    }
}

/// The code of a `STOP_SENDING` or a reset for a log line. `none`: the
/// peer's code was outside the range WebTransport gives applications, or
/// there was none.
fn stop_code(code: Option<u32>) -> String {
    code.map_or_else(|| "code=none".to_string(), |code| format!("code={code:#x}"))
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
            ..
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
        Action::OpenRequest {
            session,
            request,
            message,
        } => format!(
            "OPEN_REQUEST session={} own={} {}",
            session.0,
            request.0,
            name(message)
        ),
        Action::FinishOwnRequest { session, request } => {
            format!("FINISH_OWN_REQUEST session={} own={}", session.0, request.0)
        }
        Action::CancelOwnRequest {
            session,
            request,
            code,
        } => format!(
            "CANCEL_OWN_REQUEST session={} own={} code={code:#x}",
            session.0, request.0
        ),
        Action::PipeFetch { .. }
        | Action::AbortFetchPipe { .. }
        | Action::WriteFetchStream { .. } => return describe_fetch(action),
        // Logged where the session is closed, whoever asked for it.
        Action::CloseSession { .. }
        | Action::StartUpstreamTimer { .. }
        | Action::StopUpstreamTimer { .. }
        | Action::SendObject { .. }
        | Action::StartEarlyTimer { .. }
        | Action::StartRequestTimer { .. } => return None,
    })
}

/// The log line of an action that is about a fetch stream.
fn describe_fetch(action: &Action) -> Option<String> {
    Some(match action {
        Action::PipeFetch {
            from,
            from_stream,
            session,
            stream,
            request_id,
            ..
        } => format!(
            "PIPE_FETCH session={} stream={} request_id={request_id} from_session={} from_stream={}",
            session.0, stream.0, from.0, from_stream.0
        ),
        Action::AbortFetchPipe {
            session,
            stream,
            code,
        } => format!(
            "ABORT_FETCH_PIPE session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        Action::WriteFetchStream {
            session,
            request_id,
            body,
            ..
        } => format!(
            "WRITE_FETCH_STREAM session={} request_id={request_id} bytes={}",
            session.0,
            body.len()
        ),
        _ => return None,
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
        ControlMessage::Fetch(_) => "FETCH".to_string(),
        ControlMessage::FetchOk(_) => "FETCH_OK".to_string(),
        ControlMessage::UnsupportedRequest { message_type, .. } => {
            format!("unsupported request {message_type:#x}")
        }
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    fn timeout(session: u64, stream: u64) -> Event {
        Event::RequestTimeout {
            session: SessionKey(session),
            stream: StreamKey(stream),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_gets_only_so_many_timers() {
        // The expectations below are spelled out for a limit of 1024.
        let mut timers = Timers::default();
        let wait = Duration::from_secs(2);
        for stream in 0..1024 {
            assert!(
                timers
                    .start(SessionKey(1), 1024, wait, timeout(1, stream))
                    .is_none()
            );
        }
        assert_eq!(
            timers.start(SessionKey(1), 1024, wait, timeout(1, 1024)),
            Some(timeout(1, 1024))
        );
        // Another session is not held back by this one.
        assert!(
            timers
                .start(SessionKey(2), 1024, wait, timeout(2, 0))
                .is_none()
        );
        assert_eq!(timers.len(), 1025);

        // A timer that fired makes room for a new one.
        tokio::time::sleep(wait).await;
        timers.due().await;
        assert_eq!(timers.take_due().len(), 1025);
        assert_eq!(timers.len(), 0);
        assert!(
            timers
                .start(SessionKey(1), 1024, wait, timeout(1, 1025))
                .is_none()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn timers_fire_in_the_order_they_are_due_and_go_with_their_session() {
        let mut timers = Timers::default();
        assert!(
            timers
                .start(SessionKey(1), 8, Duration::from_secs(3), timeout(1, 0))
                .is_none()
        );
        assert!(
            timers
                .start(SessionKey(2), 8, Duration::from_secs(1), timeout(2, 0))
                .is_none()
        );
        assert!(
            timers
                .start(SessionKey(1), 8, Duration::from_secs(2), timeout(1, 1))
                .is_none()
        );

        // Nothing is due yet.
        assert_eq!(timers.take_due(), []);
        timers.due().await;
        assert_eq!(timers.take_due(), [timeout(2, 0)]);

        timers.forget_session(SessionKey(1));
        assert_eq!(timers.len(), 0);
        // With nothing running, there is nothing to wait for.
        let waited = tokio::time::timeout(Duration::from_secs(60), timers.due()).await;
        assert!(waited.is_err());
        // The session starts from nothing again.
        assert!(
            timers
                .start(SessionKey(1), 8, Duration::from_secs(1), timeout(1, 2))
                .is_none()
        );
        assert_eq!(timers.len(), 1);
    }
}
