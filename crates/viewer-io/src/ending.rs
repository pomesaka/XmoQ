//! The end of a session and of the program, for both: what stays until the
//! session is closed, how it is closed, and the signals that end the
//! program.

use std::time::Duration;

use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{Connection, IncomingStreams, client};

/// How long the relay gets to see a session closed before the next one is
/// made.
const CLOSE_WAIT: Duration = Duration::from_millis(500);

/// What has to stay until the session is closed, on every way out of the
/// loop of a session, so its caller keeps it.
///
/// WHY: a stream that is let go of is finished or stopped, and the close
/// of the session takes a moment to reach the relay. A relay that sees the
/// viewer's control stream end, or its own stopped, closes the session
/// first, with `PROTOCOL_VIOLATION` (§3.3).
#[derive(Default)]
pub struct UntilClosed {
    /// The control stream.
    pub control: Option<<WtConnection as Connection>::Send>,
    /// The readers of the streams the relay opens.
    pub readers: Option<IncomingStreams>,
    /// The readers of the request streams the relay opens.
    pub requests: Option<IncomingStreams>,
}

/// Closes the session with the code `ended` asks for, and waits a little for
/// the relay to see it.
///
/// §3.5: "`CONTROL_MESSAGE_TIMEOUT` (0x11): The session was closed because
/// the peer took too long to respond to a control message." For the SETUP,
/// as `@xmoq/moqt` has it; a request that is not answered is given up alone,
/// and the viewer then leaves with `NO_ERROR` (`termination_code` of
/// `xmoq-moqt-io`'s `client`).
pub async fn close<T, E>(connection: &WtConnection, ended: &Result<T, E>)
where
    E: AsRef<dyn std::error::Error + std::marker::Send + Sync + 'static>,
{
    connection.close(client::termination_code_of(ended), b"");
    let _ = tokio::time::timeout(CLOSE_WAIT, connection.closed()).await;
}

/// A receiver that becomes `true` at Ctrl-C, SIGTERM or SIGHUP.
#[must_use]
pub fn on_signals() -> tokio::sync::watch::Receiver<bool> {
    let (stopping, stop) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut terminate), Ok(mut hangup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
        let _ = stopping.send(true);
    });
    stop
}
