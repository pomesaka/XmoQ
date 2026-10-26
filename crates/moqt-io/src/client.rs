//! The start and the end of a client's session: the SETUP it sends and the
//! one it waits for (§3.3), what is kept until the session is closed, and
//! the code it closes the session with (§3.5). Every client of this
//! repository that is not the relay takes them from here: the Rust client
//! (`xmoq-moqt-client`), the gatherer of the chat, the transcriber, and the
//! viewers (`xmoq-viewer-io`, for the terminal viewer and the agent).
//!
//! WHY here, once: the clients start and close a session the same way, and
//! copies of the rule had drifted apart (one found a SETUP that did not come
//! under a `context`, the others did not, and closed with `NO_ERROR`).

use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc;
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::setup_option;
use xmoq_moqt::session::{ControlEvent, Session};
use xmoq_moqt::{TerminationCode, UsageError};

use crate::{
    Connection, ConnectionLost, IncomingStreams, SendError, Trace, UniEvent, send_message,
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
pub struct UntilClosed<C: Connection> {
    /// The control stream.
    pub control: Option<C::Send>,
    /// The readers of the peer's streams.
    pub readers: Vec<IncomingStreams>,
}

impl<C: Connection> Default for UntilClosed<C> {
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
/// ([`HeldTooMuch`]); the one a protocol error asks for; and `NO_ERROR` for
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
    if causes().any(<dyn std::error::Error>::is::<HeldTooMuch>) {
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
    use xmoq_moqt::message::{ControlMessage, RequestOk, Setup};
    use xmoq_moqt::session::{Role, Transport};

    use super::*;
    use crate::memory::{MemoryConnection, pair};
    use crate::{SendStream as _, StreamConfig, StreamStops, read_uni_streams};

    #[test]
    fn a_session_is_closed_with_the_code_its_failure_asks_for() {
        // (what failed; the code the session is closed with)
        let cases: [(Box<dyn std::error::Error>, u32); 7] = [
            (Box::new(SetupError::NoSetup("the readers stopped")), 0x0),
            (Box::new(NoSetupInTime(SETUP_WAIT)), 0x11),
            (Box::new(SetupError::HeldTooMuch(HeldTooMuch)), 0x1),
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
