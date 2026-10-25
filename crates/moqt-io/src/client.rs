//! The start and the end of a client's session: the SETUP it sends and the
//! one it waits for (§3.3), what is kept until the session is closed, and
//! the code it closes the session with (§3.5). The Rust client
//! (`xmoq-moqt-client`), the gatherer of the chat and the transcriber take
//! them from here.
//!
//! WHY here, once: the clients start and close a session the same way, and
//! copies of the rule had drifted apart (one found a SETUP that did not come
//! under a `context`, the others did not, and closed with `NO_ERROR`).

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
    /// Something else came first, or the readers stopped before a SETUP.
    NoSetup(&'static str),
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

/// Opens the control stream, sends SETUP with `options`, and hands the
/// first message of the peer's control stream to `session`, which takes a
/// SETUP alone there. `events` are those of [`crate::read_uni_streams`].
/// The control stream goes into `control` as soon as it is open, also where
/// what follows fails ([`UntilClosed`]).
///
/// # Errors
///
/// [`SetupError`]; [`termination_code`] gives the code to close with.
pub async fn setup<C: Connection>(
    connection: &C,
    session: &mut Session,
    events: &mut mpsc::Receiver<UniEvent>,
    control: &mut Option<C::Send>,
    options: Vec<KeyValuePair>,
    wait: Duration,
) -> Result<ControlEvent, SetupError> {
    let control = control.insert(connection.open_uni().await.map_err(SetupError::Lost)?);
    let setup = session.setup(options).map_err(SetupError::Usage)?;
    send_message(control, &setup, &Trace::default(), "control")
        .await
        .map_err(SetupError::Send)?;
    match tokio::time::timeout(wait, events.recv()).await {
        Ok(Some(UniEvent::Control(message))) => session
            .on_control_message(message)
            .map_err(SetupError::Session),
        // WHY the error itself: the session is closed with its code, as a
        // datagram of an unknown type that comes before the SETUP has it
        // (§11).
        Ok(Some(UniEvent::Failed(error))) => Err(SetupError::Session(error)),
        Ok(Some(_)) => Err(SetupError::NoSetup("a stream of data came first")),
        Ok(None) => Err(SetupError::NoSetup("the readers of the streams stopped")),
        Err(_) => Err(SetupError::TimedOut(NoSetupInTime(wait))),
    }
}

/// The code to close the session with after `error` (§3.5):
/// `CONTROL_MESSAGE_TIMEOUT` for a SETUP that did not come, as `@xmoq/moqt`
/// has it; the one a protocol error asks for; and `NO_ERROR` for everything
/// else, a request that is given up alone among them.
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

    use xmoq_moqt::message::{ControlMessage, RequestOk, Setup};
    use xmoq_moqt::session::{Role, Transport};

    use super::*;
    use crate::memory::{MemoryConnection, pair};
    use crate::{SendStream as _, StreamConfig, StreamStops, read_uni_streams};

    #[test]
    fn a_session_is_closed_with_the_code_its_failure_asks_for() {
        // (what failed; the code the session is closed with)
        let cases: [(Box<dyn std::error::Error>, u32); 6] = [
            (Box::new(SetupError::NoSetup("the readers stopped")), 0x0),
            (Box::new(NoSetupInTime(SETUP_WAIT)), 0x11),
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
        let (queue, events) = mpsc::channel(16);
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
    }

    async fn peer_says(peer: &MemoryConnection, message: &ControlMessage) {
        let mut bytes = Vec::new();
        message.encode(&mut bytes).unwrap();
        let mut control = peer.open_uni().await.unwrap();
        control.write_all(&bytes).await.unwrap();
        // The peer's control stream stays open (§3.3).
        std::mem::forget(control);
    }

    #[tokio::test(start_paused = true)]
    async fn the_peers_setup_is_waited_for() {
        // (what the peer does first; the code of the failure, or `None`
        // where the session was set up)
        let cases: [(First, Option<u32>); 5] = [
            (First::Says(ControlMessage::Setup(Setup::default())), None),
            (First::Nothing, Some(0x11)),
            (
                First::Says(ControlMessage::RequestOk(RequestOk::default())),
                Some(0x3),
            ),
            (First::SendsADatagram, Some(0x3)),
            (First::ReadersStop, Some(0x0)),
        ];
        for (first, code) in cases {
            let mut client = client();
            match &first {
                First::Says(message) => peer_says(&client.peer, message).await,
                First::Nothing => {}
                First::SendsADatagram => client.peer.send_datagram(&[]),
                First::ReadersStop => client.readers = None,
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
                None => assert!(
                    matches!(result, Ok(ControlEvent::Setup(_))),
                    "{first:?}: {result:?}"
                ),
                Some(code) => {
                    let error = result.unwrap_err();
                    assert_eq!(termination_code(&error), code, "{first:?}: {error}");
                }
            }
        }
    }
}
