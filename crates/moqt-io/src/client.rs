//! The start and the end of a client's session: the SETUP it sends and the
//! one it waits for (§3.3), what is kept until the session is closed, the
//! code it closes the session with (§3.5) and how long it waits for the
//! close to reach the peer, the GOAWAY behind a refusal (§10.4), and the
//! signals that end the program. Every client of this repository that is
//! not the relay takes them from here: the gatherer of the chat, the
//! transcriber, the viewers (`xmoq-viewer-io`, for the terminal viewer and
//! the agent) and the Rust client (`xmoq-moqt-client`), which waits for the
//! peer's SETUP itself (why, where it does).
//!
//! WHY here, once: the clients start and close a session the same way, and
//! a copy of the rule in each would part from the others (a SETUP that did
//! not come is found under any number of wrappers here, [`termination_code`]).

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{GoAway, request_error_code, setup_option};
use xmoq_moqt::session::{ControlEvent, Session};
use xmoq_moqt::{TerminationCode, UsageError};

use crate::{
    Connection, ConnectionEnd, ConnectionLost, IncomingStreams, SendError, Trace, UniEvent,
    send_message,
};

/// How long the peer's SETUP is waited for, from when this client's was
/// written, in milliseconds. **Provisional**, and the time `@xmoq/moqt`
/// gives it.
pub const SETUP_WAIT_MS: u64 = 10_000;
/// [`SETUP_WAIT_MS`] as a [`Duration`].
pub const SETUP_WAIT: Duration = Duration::from_millis(SETUP_WAIT_MS);

/// The events of the peer's data streams that a client holds while it waits
/// for the peer's SETUP ([`setup`]); one more ends the wait
/// ([`HeldTooMuch`]). **Provisional**, decided on 2026-10-26. WHY
/// held: §3.3, "Unidirectional streams containing Objects ... could arrive
/// prior to the control streams, in which case the data SHOULD be buffered
/// until both control streams arrive and setup is complete". WHY a number
/// of events and not of bytes: each holds no more than one stream may
/// (`StreamConfig`), and a client has asked for nothing before its session
/// is set up, so that a peer that sends more than a few is not one to wait
/// for.
pub const MOST_HELD_BEFORE_SETUP: usize = 64;

/// What [`setup`] got.
#[derive(Debug)]
pub struct SetUp {
    /// What the session made of the peer's SETUP.
    pub peer: ControlEvent,
    /// The events of the peer's data streams that came before it, in their
    /// order, for the client to take before any other
    /// ([`MOST_HELD_BEFORE_SETUP`] at most).
    pub early: Vec<UniEvent>,
}

/// The peer's data streams brought more before its SETUP than a client
/// holds ([`MOST_HELD_BEFORE_SETUP`]).
///
/// WHY the session is closed with `INTERNAL_ERROR` (§3.5: "An
/// implementation specific error occurred"): a limit of this endpoint was
/// reached, as when the relay is full (`HubConfig::max_sessions` of
/// `xmoq-relay`). WHY NOT `EXCESSIVE_LOAD`: §3.5 has no such Session
/// Termination Error Code (it is a code of a stream reset, of `REQUEST_ERROR`
/// and of `PUBLISH_DONE`), and its value, 0x9, is `MALFORMED_PATH` there.
/// WHY NOT reset the streams beyond the limit and go on: §3.3 lets an
/// endpoint that "does not want to buffer" reset only "such bidirectional
/// streams", and a client has asked for nothing such a stream could carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldTooMuch;

impl std::fmt::Display for HeldTooMuch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the peer's data streams brought more than {MOST_HELD_BEFORE_SETUP} events before its SETUP"
        )
    }
}

impl std::error::Error for HeldTooMuch {}

/// A reader of this crate panicked and lost a stream of the peer
/// (`UniEvent::ReaderPanicked`, `PeerRequestStreamEvent::ReaderPanicked`):
/// the session cannot go on with a hole in it, and is closed with
/// `INTERNAL_ERROR` ([`termination_code`]), the fault being on this side
/// (§3.5: "An implementation specific error occurred").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderPanicked;

impl std::fmt::Display for ReaderPanicked {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "a reader of a stream of the peer panicked")
    }
}

impl std::error::Error for ReaderPanicked {}

/// The peer sent no SETUP within the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoSetupInTime(pub Duration);

impl std::fmt::Display for NoSetupInTime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the peer sent no SETUP within {} ms",
            self.0.as_millis()
        )
    }
}

impl std::error::Error for NoSetupInTime {}

/// Why [`setup`] failed.
#[derive(Debug)]
pub enum SetupError {
    /// The control stream could not be opened.
    Lost(ConnectionLost),
    /// This client's SETUP could not be made.
    Usage(UsageError),
    /// It could not be written.
    Send(SendError),
    /// The session failed before the peer's SETUP, or on what came first:
    /// to be closed with its code.
    Session(xmoq_moqt::Error),
    /// The readers stopped before a SETUP.
    NoSetup(&'static str),
    /// More data came first than is held.
    HeldTooMuch(HeldTooMuch),
    /// None came in time.
    TimedOut(NoSetupInTime),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lost(error) => write!(formatter, "opening the control stream: {error}"),
            Self::Usage(error) => write!(formatter, "making the SETUP: {error}"),
            Self::Send(error) => write!(formatter, "sending the SETUP: {error}"),
            Self::Session(error) => write!(formatter, "before the peer's SETUP: {error}"),
            Self::NoSetup(what) => write!(formatter, "no SETUP from the peer: {what}"),
            Self::TimedOut(error) => write!(formatter, "{error}"),
            Self::HeldTooMuch(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for SetupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lost(error) => Some(error),
            Self::Usage(error) => Some(error),
            Self::Send(error) => Some(error),
            Self::Session(error) => Some(error),
            Self::NoSetup(_) => None,
            Self::TimedOut(error) => Some(error),
            Self::HeldTooMuch(error) => Some(error),
        }
    }
}

/// The options of a client's SETUP: `token` as `AUTHORIZATION_TOKEN`, where
/// there is one, and `implementation` as `MOQT_IMPLEMENTATION`, in
/// ascending order of their types (0x03, then 0x07).
///
/// # Errors
///
/// None for these types, which are odd; the error is the encoder's.
pub fn setup_options(
    token: Option<&[u8]>,
    implementation: &str,
) -> Result<Vec<KeyValuePair>, UsageError> {
    let mut options = Vec::new();
    if let Some(token) = token {
        options.push(KeyValuePair::bytes(
            setup_option::AUTHORIZATION_TOKEN,
            token.to_vec(),
        )?);
    }
    options.push(KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        implementation.as_bytes().to_vec(),
    )?);
    Ok(options)
}

/// What a client keeps until its session is closed, also where the session
/// failed: its control stream, and the readers of the peer's streams.
///
/// WHY: a peer that sees the control stream end closes the session with
/// `PROTOCOL_VIOLATION` first (§3.3), and with `web-transport-quinn` a send
/// stream that is dropped is finished (quinn 0.11.12, `Drop for
/// SendStream`); a stream that is no longer read is stopped. Kept, the
/// session is closed with the code its failure asks for.
pub struct UntilClosed<C: Connection, R = IncomingStreams> {
    /// The control stream.
    pub control: Option<C::Send>,
    /// The readers of the peer's streams: this crate's, or a client's own
    /// that hold them (the Rust client's).
    pub readers: Vec<R>,
}

impl<C: Connection, R> Default for UntilClosed<C, R> {
    fn default() -> Self {
        Self {
            control: None,
            readers: Vec::new(),
        }
    }
}

/// Where a client takes the events of the peer's unidirectional streams
/// from: the channel of [`crate::read_uni_streams`], or a client's own
/// queue in front of it.
pub trait UniEvents {
    /// The next event; `None` once the readers are gone.
    fn next_event(&mut self) -> impl Future<Output = Option<UniEvent>> + Send;
}

impl UniEvents for mpsc::Receiver<UniEvent> {
    fn next_event(&mut self) -> impl Future<Output = Option<UniEvent>> + Send {
        self.recv()
    }
}

/// Events that can be given back, to come again before what is still to
/// come: what a wait read and did not take ([`go_away_behind`]).
pub trait PutBack: UniEvents {
    /// Puts `kept`, read and not taken, before what comes next, in its
    /// order.
    fn put_back(&mut self, kept: VecDeque<UniEvent>);
}

/// The events of a channel with those given back in front of them.
#[derive(Debug)]
pub struct ReadAhead {
    received: mpsc::Receiver<UniEvent>,
    ahead: VecDeque<UniEvent>,
}

impl ReadAhead {
    /// The events of `received`.
    #[must_use]
    pub fn new(received: mpsc::Receiver<UniEvent>) -> Self {
        Self {
            received,
            ahead: VecDeque::new(),
        }
    }
}

impl UniEvents for ReadAhead {
    async fn next_event(&mut self) -> Option<UniEvent> {
        match self.ahead.pop_front() {
            Some(event) => Some(event),
            None => self.received.recv().await,
        }
    }
}

impl PutBack for ReadAhead {
    fn put_back(&mut self, mut kept: VecDeque<UniEvent>) {
        kept.append(&mut self.ahead);
        self.ahead = kept;
    }
}

/// The most events [`go_away_behind`] keeps while it waits: past them the
/// wait ends as one in which no GOAWAY came. **Provisional.**
pub const MOST_READ_AHEAD: usize = 256;

/// The GOAWAY behind a request refused with `refusal_code`, where that is
/// `GOING_AWAY` (§10.4): the GOAWAY travels on the control stream and the
/// refusal on the request's, between which QUIC keeps no order, so the
/// refusal can be read first. `None` for any other code, where no GOAWAY
/// comes within `wait`, and where the connection is gone. WHY a limit, the
/// time an answer gets: `GOING_AWAY` also comes from a peer that received a
/// GOAWAY (§10.6.2) and sends none. WHY `None` for a connection that is
/// gone and no error: the caller's loop finds that at once, and
/// `goAwayBehind` of `@xmoq/moqt` says the same; both are tested with
/// `fixtures/ask-again/go-away-behind.tsv`.
///
/// What else comes meanwhile is given back to `events` ([`PutBack`]),
/// before what comes after, in its order: where no GOAWAY comes the session
/// goes on, and an Object of another track read in the wait is still that
/// track's. Control messages are taken by `session`, as the caller's loop
/// would.
///
/// # Errors
///
/// A control message that breaks the session.
pub async fn go_away_behind<E: PutBack>(
    session: &mut Session,
    events: &mut E,
    refusal_code: u64,
    wait: Duration,
) -> Result<Option<GoAway>, xmoq_moqt::Error> {
    if refusal_code != request_error_code::GOING_AWAY {
        return Ok(None);
    }
    let until = tokio::time::Instant::now() + wait;
    let mut kept = VecDeque::new();
    let found = loop {
        if kept.len() >= MOST_READ_AHEAD {
            break Ok(None);
        }
        let Ok(event) = tokio::time::timeout_at(until, events.next_event()).await else {
            break Ok(None);
        };
        match event {
            Some(UniEvent::Control(message)) => match session.on_control_message(message) {
                Ok(ControlEvent::GoAway(go_away)) => break Ok(Some(go_away)),
                Ok(_) => {}
                Err(error) => break Err(error),
            },
            Some(event) => kept.push_back(event),
            None => break Ok(None),
        }
    };
    events.put_back(kept);
    found
}

/// How long a client waits, after it closed its session, for the close to
/// reach the peer, before it goes on or exits: a process that exits before
/// that leaves the peer with a lost connection. Each role has its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseWait(pub Duration);

impl CloseWait {
    /// The terminal viewer's and the agent's: 500 ms. **Provisional.** WHY
    /// shorter: a person waits for the next session.
    pub const VIEWER: Self = Self(Duration::from_millis(500));

    /// The gatherer's, the transcriber's and the Rust client's: 2 s.
    /// **Provisional.**
    pub const SERVING: Self = Self(Duration::from_secs(2));
}

/// Closes the session with the code `ended` asks for, and waits `wait` at
/// most for the close to reach the peer.
///
/// §3.5: "`CONTROL_MESSAGE_TIMEOUT` (0x11): The session was closed because
/// the peer took too long to respond to a control message." For the SETUP,
/// as `@xmoq/moqt` has it; a request that is not answered is given up
/// alone, and the client then leaves with `NO_ERROR`
/// ([`termination_code`]).
pub async fn close<C: Connection, T, E>(connection: &C, ended: &Result<T, E>, wait: CloseWait)
where
    E: AsRef<dyn std::error::Error + Send + Sync + 'static>,
{
    connection.close(termination_code_of(ended), b"");
    let _ = tokio::time::timeout(wait.0, connection.closed()).await;
}

/// How long a client whose session failed waits to learn whether the peer
/// closed it, and with what ([`peer_close`]). **Provisional.**
pub const PEER_CLOSE_WAIT: Duration = Duration::from_millis(500);

/// How the connection ended where it ends within [`PEER_CLOSE_WAIT`]. WHY:
/// what failed first in a client is one of several things (a stream that
/// ended, a read that was lost), and none of them says why; the connection
/// does, a moment later.
pub async fn peer_close<C: Connection>(connection: &C) -> Option<ConnectionEnd> {
    tokio::time::timeout(PEER_CLOSE_WAIT, connection.closed())
        .await
        .ok()
}

/// A session on `connection` from its start to its close: `run` is given
/// what has to stay until the session is closed ([`UntilClosed`]); where it
/// fails, how the peer ended the connection is waited for ([`peer_close`]);
/// then the session is closed with the code its end asks for, and the close
/// waited for by `wait` ([`close`]). Returns what `run` returned, with how
/// the peer ended the connection where it failed and the peer did.
pub async fn on_connection<C, R, T, E>(
    connection: &Arc<C>,
    wait: CloseWait,
    run: impl AsyncFnOnce(&Arc<C>, &mut UntilClosed<C, R>) -> Result<T, E>,
) -> (Result<T, E>, Option<ConnectionEnd>)
where
    C: Connection,
    E: AsRef<dyn std::error::Error + Send + Sync + 'static>,
{
    let mut kept = UntilClosed::default();
    let ran = run(connection, &mut kept).await;
    let peer = match &ran {
        Ok(_) => None,
        Err(_) => peer_close(connection.as_ref()).await,
    };
    close(connection.as_ref(), &ran, wait).await;
    drop(kept);
    (ran, peer)
}

/// The signals a program ends at. Each role has its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signals {
    /// Whether SIGHUP is one.
    pub hangup: bool,
}

impl Signals {
    /// The terminal viewer's and the agent's: Ctrl-C, SIGTERM and SIGHUP.
    /// WHY SIGHUP: they run in a terminal, and end with it.
    pub const VIEWER: Self = Self { hangup: true };

    /// The gatherer's and the transcriber's: Ctrl-C and SIGTERM. WHY NOT
    /// SIGHUP: they serve the room, and a terminal they were started from
    /// that closes is not told to end them.
    pub const SERVING: Self = Self { hangup: false };
}

/// A receiver that becomes `true` at one of `signals`.
#[must_use]
pub fn on_signals(signals: Signals) -> watch::Receiver<bool> {
    let (stopping, stop) = watch::channel(false);
    tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let Ok(mut terminate) = signal(SignalKind::terminate()) else {
            return;
        };
        let mut hangup = if signals.hangup {
            match signal(SignalKind::hangup()) {
                Ok(hangup) => Some(hangup),
                Err(_) => return,
            }
        } else {
            None
        };
        let hung_up = async {
            match hangup.as_mut() {
                Some(hangup) => hangup.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
            _ = hung_up => {}
        }
        let _ = stopping.send(true);
    });
    stop
}

/// Opens the control stream, sends SETUP with `options`, and hands the
/// first message of the peer's control stream to `session`, which takes a
/// SETUP alone there. The events of data streams that come before it are
/// held and returned with it ([`SetUp::early`]).
/// The control stream goes into `control` as soon as it is open, also where
/// what follows fails ([`UntilClosed`]).
///
/// # Errors
///
/// [`SetupError`]; [`termination_code`] gives the code to close with.
pub async fn setup<C: Connection, E: UniEvents>(
    connection: &C,
    session: &mut Session,
    events: &mut E,
    control: &mut Option<C::Send>,
    options: Vec<KeyValuePair>,
    wait: Duration,
) -> Result<SetUp, SetupError> {
    let control = control.insert(connection.open_uni().await.map_err(SetupError::Lost)?);
    let setup = session.setup(options).map_err(SetupError::Usage)?;
    send_message(control, &setup, &Trace::default(), "control")
        .await
        .map_err(SetupError::Send)?;
    let deadline = tokio::time::Instant::now() + wait;
    let mut early = Vec::new();
    loop {
        match tokio::time::timeout_at(deadline, events.next_event()).await {
            Ok(Some(UniEvent::Control(message))) => {
                let peer = session
                    .on_control_message(message)
                    .map_err(SetupError::Session)?;
                return Ok(SetUp { peer, early });
            }
            // WHY the error itself: the session is closed with its code, as
            // a datagram of an unknown type that comes before the SETUP has
            // it (§11).
            Ok(Some(UniEvent::Failed(error))) => return Err(SetupError::Session(error)),
            Ok(Some(event)) => {
                if early.len() == MOST_HELD_BEFORE_SETUP {
                    return Err(SetupError::HeldTooMuch(HeldTooMuch));
                }
                early.push(event);
            }
            Ok(None) => return Err(SetupError::NoSetup("the readers of the streams stopped")),
            Err(_) => return Err(SetupError::TimedOut(NoSetupInTime(wait))),
        }
    }
}

/// The code to close the session with after `error` (§3.5):
/// `CONTROL_MESSAGE_TIMEOUT` for a SETUP that did not come, as `@xmoq/moqt`
/// has it; `INTERNAL_ERROR` for more data before it than is held
/// ([`HeldTooMuch`]) and for a reader that panicked ([`ReaderPanicked`]);
/// the one a protocol error asks for; and `NO_ERROR` for
/// everything else, a request that is given up alone among them.
///
/// WHY the chain of sources: a wrapper (`anyhow`'s `context`, a
/// [`SetupError`]) hides the cause, which may be under any number of them.
/// An `anyhow::Error` is passed as `error.as_ref()`.
#[must_use]
pub fn termination_code(error: &(dyn std::error::Error + 'static)) -> u32 {
    let causes = || std::iter::successors(Some(error), |cause| cause.source());
    if causes().any(<dyn std::error::Error>::is::<NoSetupInTime>) {
        return TerminationCode::ControlMessageTimeout.code();
    }
    if causes().any(|cause| cause.is::<HeldTooMuch>() || cause.is::<ReaderPanicked>()) {
        return TerminationCode::InternalError.code();
    }
    causes()
        .find_map(|cause| cause.downcast_ref::<xmoq_moqt::Error>())
        .and_then(xmoq_moqt::Error::termination_code)
        .map_or(0, TerminationCode::code)
}

/// The code to close the session with after it ended with `result`:
/// `NO_ERROR` where it ended without an error, [`termination_code`]
/// otherwise. `E` is `anyhow::Error`, say.
#[must_use]
pub fn termination_code_of<T, E>(result: &Result<T, E>) -> u32
where
    E: AsRef<dyn std::error::Error + Send + Sync + 'static>,
{
    result
        .as_ref()
        .map_or_else(|error| termination_code(error.as_ref()), |_| 0)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use xmoq_moqt::data::{
        ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
    };
    use xmoq_moqt::message::{ControlMessage, GoAway, RequestOk, Setup};
    use xmoq_moqt::session::{Role, Transport};

    use super::*;
    use crate::memory::{MemoryConnection, pair};
    use crate::{SendStream as _, StreamConfig, StreamStops, read_uni_streams};

    #[test]
    fn a_session_is_closed_with_the_code_its_failure_asks_for() {
        // (what failed; the code the session is closed with)
        let cases: [(Box<dyn std::error::Error>, u32); 8] = [
            (Box::new(SetupError::NoSetup("the readers stopped")), 0x0),
            (Box::new(NoSetupInTime(SETUP_WAIT)), 0x11),
            (Box::new(SetupError::HeldTooMuch(HeldTooMuch)), 0x1),
            (Box::new(ReaderPanicked), 0x1),
            (
                Box::new(SetupError::TimedOut(NoSetupInTime(SETUP_WAIT))),
                0x11,
            ),
            (Box::new(xmoq_moqt::Error::ProtocolViolation("first")), 0x3),
            (
                Box::new(SetupError::Session(xmoq_moqt::Error::DuplicateTrackAlias(
                    7,
                ))),
                0x5,
            ),
            (
                Box::new(SetupError::Lost(ConnectionLost("gone".to_owned()))),
                0x0,
            ),
        ];
        for (error, code) in cases {
            assert_eq!(termination_code(error.as_ref()), code, "{error}");
        }
    }

    /// The rows of `fixtures/ask-again/go-away-behind.tsv`, which
    /// `goAwayBehind` of `@xmoq/moqt` is tested with too.
    #[tokio::test(start_paused = true)]
    async fn the_goaway_behind_a_refusal_is_as_the_shared_table_says() {
        let path = format!(
            "{}/../../fixtures/ask-again/go-away-behind.tsv",
            env!("CARGO_MANIFEST_DIR")
        );
        let table =
            std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        assert!(table.ends_with('\n') && !table.ends_with("\n\n"), "{path}");
        let rows: Vec<Vec<&str>> = table.lines().map(|row| row.split('\t').collect()).collect();
        // A table that lost rows would pass.
        assert_eq!(rows.len(), 6, "{path}");
        for row in rows {
            let [case, refusal, comes, wait_ms, expected] = row.as_slice() else {
                panic!("a row of five columns: {row:?}");
            };
            let refusal_code = match *refusal {
                "GOING_AWAY" => request_error_code::GOING_AWAY,
                "NOT_SUPPORTED" => request_error_code::NOT_SUPPORTED,
                other => panic!("a code the test does not know: {other}"),
            };
            let mut session = Session::new(Role::Client, Transport::WebTransport);
            session
                .on_control_message(ControlMessage::Setup(Setup {
                    setup_options: vec![],
                }))
                .unwrap();
            let (send, received) = mpsc::channel(16);
            let steps: Vec<(String, u64)> = comes
                .split(' ')
                .filter(|step| *step != "-")
                .map(|step| {
                    let (what, at) = step.split_once('@').expect("what@T");
                    (what.to_string(), at.parse().expect("a time"))
                })
                .collect();
            let objects = steps.iter().filter(|(what, _)| what == "object").count();
            tokio::spawn(async move {
                let began = tokio::time::Instant::now();
                for (what, at) in steps {
                    tokio::time::sleep_until(began + Duration::from_millis(at)).await;
                    let event = match what.as_str() {
                        "goaway" => UniEvent::Control(ControlMessage::GoAway(GoAway {
                            new_session_uri: b"https://b/".to_vec(),
                            timeout_ms: 0,
                            request_id: Some(0),
                        })),
                        "object" => UniEvent::SubgroupOpened {
                            stream: at,
                            header: SubgroupHeader {
                                track_alias: 0,
                                group_id: at,
                                subgroup_id: SubgroupId::Zero,
                                publisher_priority: None,
                                properties: false,
                                end_of_group: false,
                                first_object: false,
                            },
                        },
                        // The readers are gone, and the channel with them.
                        "closed" => return,
                        other => panic!("a step the test does not know: {other}"),
                    };
                    let _ = send.send(event).await;
                }
                // Nothing more comes, and the connection stays.
                std::future::pending::<()>().await;
            });
            let mut events = ReadAhead::new(received);
            let wait = Duration::from_millis(wait_ms.parse().expect("a wait"));
            let got = go_away_behind(&mut session, &mut events, refusal_code, wait)
                .await
                .unwrap();
            let got = if got.is_some() { "goaway" } else { "none" };
            assert_eq!(got, *expected, "{case}");
            // What came before the GOAWAY comes again, in its order.
            if *expected == "goaway" {
                for index in 0..objects {
                    let again = events.ahead.get(index);
                    assert!(
                        matches!(again, Some(UniEvent::SubgroupOpened { .. })),
                        "{case}: {again:?}"
                    );
                }
                assert_eq!(events.ahead.len(), objects, "{case}");
            }
        }
    }

    /// A client's side of `setup`, with the peer the test writes as.
    struct Client {
        connection: Arc<MemoryConnection>,
        peer: MemoryConnection,
        session: Session,
        events: mpsc::Receiver<UniEvent>,
        readers: Option<IncomingStreams>,
    }

    fn client() -> Client {
        let (local, peer) = pair();
        let local = Arc::new(local);
        // Room for every event a case sends before `setup` reads one: a
        // full queue would hold the peer's writes up for ever.
        let (queue, events) = mpsc::channel(128);
        let readers = read_uni_streams(
            Arc::clone(&local),
            StreamConfig::default(),
            StreamStops::default(),
            queue,
            |event| event,
        );
        Client {
            connection: local,
            peer,
            session: Session::new(Role::Client, Transport::WebTransport),
            events,
            readers: Some(readers),
        }
    }

    /// What the peer does before the client's SETUP is waited for.
    #[derive(Debug)]
    enum First {
        Says(ControlMessage),
        Nothing,
        /// A datagram without a type, which fails the session (§11).
        SendsADatagram,
        /// The readers of the client stop.
        ReadersStop,
        /// A data stream with this many Objects, ended, and then a SETUP.
        DataThenSetup(u64),
    }

    async fn peer_says(peer: &MemoryConnection, message: &ControlMessage) {
        let mut bytes = Vec::new();
        message.encode(&mut bytes).unwrap();
        let mut control = peer.open_uni().await.unwrap();
        control.write_all(&bytes).await.unwrap();
        // The peer's control stream stays open (§3.3).
        std::mem::forget(control);
    }

    /// A data stream of `objects` Objects, ended: as many events and two.
    async fn peer_sends_a_subgroup(peer: &MemoryConnection, objects: u64) {
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias: 7,
            group_id: 0,
            subgroup_id: SubgroupId::Zero,
            publisher_priority: Some(0),
            properties: false,
            end_of_group: false,
            first_object: true,
        });
        for object_id in 0..objects {
            encoder
                .object(&SubgroupObject {
                    object_id,
                    properties: vec![],
                    status: ObjectStatus::Normal,
                    payload: vec![0x61],
                })
                .unwrap();
        }
        let mut stream = peer.open_uni().await.unwrap();
        stream.write_all(&encoder.take_bytes()).await.unwrap();
        stream.finish();
    }

    #[tokio::test(start_paused = true)]
    async fn the_peers_setup_is_waited_for() {
        // (what the peer does first; the code of the failure, or the
        // events held before the SETUP where the session was set up)
        let cases: [(First, Result<usize, u32>); 8] = [
            (First::Says(ControlMessage::Setup(Setup::default())), Ok(0)),
            // §3.3: data before the SETUP is held, and given after it.
            (First::DataThenSetup(1), Ok(3)),
            (First::DataThenSetup(62), Ok(64)),
            // One event beyond `MOST_HELD_BEFORE_SETUP`.
            (First::DataThenSetup(63), Err(0x1)),
            (First::Nothing, Err(0x11)),
            (
                First::Says(ControlMessage::RequestOk(RequestOk::default())),
                Err(0x3),
            ),
            (First::SendsADatagram, Err(0x3)),
            (First::ReadersStop, Err(0x0)),
        ];
        for (first, code) in cases {
            let mut client = client();
            match &first {
                First::Says(message) => peer_says(&client.peer, message).await,
                First::Nothing => {}
                First::SendsADatagram => client.peer.send_datagram(&[]),
                First::ReadersStop => client.readers = None,
                First::DataThenSetup(objects) => {
                    peer_sends_a_subgroup(&client.peer, *objects).await;
                    // The readers take the data stream before the control
                    // stream is there.
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    peer_says(&client.peer, &ControlMessage::Setup(Setup::default())).await;
                }
            }
            let mut control = None;
            let options = setup_options(Some(b"token"), "test").unwrap();
            let result = setup(
                client.connection.as_ref(),
                &mut client.session,
                &mut client.events,
                &mut control,
                options,
                SETUP_WAIT,
            )
            .await;
            assert!(control.is_some(), "{first:?}: the control stream is kept");
            match code {
                Ok(held) => {
                    let set_up = result.unwrap();
                    assert!(matches!(set_up.peer, ControlEvent::Setup(_)), "{first:?}");
                    assert_eq!(set_up.early.len(), held, "{first:?}");
                    assert!(
                        set_up.early.iter().all(|event| matches!(
                            event,
                            UniEvent::SubgroupOpened { .. }
                                | UniEvent::Object { .. }
                                | UniEvent::SubgroupClosed { .. }
                        )),
                        "{first:?}"
                    );
                }
                Err(code) => {
                    let error = result.unwrap_err();
                    assert_eq!(termination_code(&error), code, "{first:?}: {error}");
                }
            }
        }
    }
}
