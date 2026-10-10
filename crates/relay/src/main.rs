//! The `XmoQ` relay: `MoQ` Transport draft 18 over WebTransport.
//!
//! It takes PUBLISH from publishers and SUBSCRIBE from subscribers and forwards
//! the Objects of each track to its subscribers. It keeps no Objects: a
//! subscriber gets what arrives after it subscribed.
//!
//! Provisional, for testing only: there is no authentication or authorization,
//! so anyone who can reach the port can publish and subscribe.
//!
//! What is decided where: `xmoq-relay-core` decides who gets what and who is
//! cut off, `xmoq-moqt-io` reads and writes the streams, and this crate
//! connects the two.

mod hub;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::Semaphore;
use xmoq_moqt_io::StreamConfig;
use xmoq_moqt_io::wt::{QuicLimits, WtConnection, WtServer, accept};

use crate::hub::{HubConfig, HubHandle, Log};

/// How long a client may take over the WebTransport handshake. Provisional.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Handshakes in progress at once. Further connection attempts wait to be
/// taken, in the queue of the QUIC endpoint. Provisional.
///
/// WHY: the limit on sessions only counts a connection once its handshake is
/// done, so without this one a client could keep any number of half-open
/// connections, each with a task, for the length of the timeout.
const MAX_HANDSHAKES: usize = 64;

/// The QUIC limits that go with the limits of the relay. Provisional.
///
/// Three limits act on the requests a peer opens, and this is the one place
/// that sets how they relate:
///
/// 1. `MAX_PEER_REQUESTS` (256) of the session: a request beyond it is
///    answered with `EXCESSIVE_LOAD`.
/// 2. `max_request_streams` (264) of the stream handling: that many request
///    streams are read at once. It is above 1, so that the answer of 1 can be
///    given.
/// 3. The stream limit of QUIC, set to 2 here: a peer cannot open more
///    streams than are read, and waits in its own QUIC stack instead.
///
/// So 1 acts first, and a peer that ignores `EXCESSIVE_LOAD` is held at 3.
/// With quinn's default of 100 bidirectional streams, 3 would act first and
/// `EXCESSIVE_LOAD` would never be sent. The same goes for unidirectional
/// streams, where the stream handling reads 64 at once.
///
/// The windows bound what the QUIC layer holds for one peer; `HubConfig` adds
/// them up with the rest.
fn quic_limits(streams: &StreamConfig, idle_timeout: Duration) -> QuicLimits {
    const STREAM_RECEIVE_WINDOW: u32 = 256 * 1024;
    const RECEIVE_WINDOW: u32 = 4 * 1024 * 1024;
    const SEND_WINDOW: u32 = 4 * 1024 * 1024;
    let (max_uni_streams, max_bi_streams) = xmoq_moqt_io::wt::streams_read_at_once(streams);
    QuicLimits {
        max_uni_streams,
        max_bi_streams,
        stream_receive_window: STREAM_RECEIVE_WINDOW,
        receive_window: RECEIVE_WINDOW,
        send_window: SEND_WINDOW,
        idle_timeout,
    }
}

struct Args {
    port: u16,
    cert_file: String,
    key_file: String,
    /// How long nothing may arrive from a peer before its connection counts
    /// as lost. A run that waits for the relay to notice a vanished peer
    /// sets it lower than the default.
    idle_timeout: Duration,
}

fn parse_args() -> Result<Args> {
    let mut port = None;
    let mut cert_file = None;
    let mut key_file = None;
    let mut idle_timeout = xmoq_moqt_io::wt::IDLE_TIMEOUT;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| anyhow!("missing value for {flag}"))?;
        match flag.as_str() {
            "--port" => port = Some(value.parse().context("--port")?),
            "--cert-file" => cert_file = Some(value),
            "--key-file" => key_file = Some(value),
            "--idle-timeout-ms" => {
                let millis: u64 = value.parse().context("--idle-timeout-ms")?;
                if millis == 0 {
                    bail!("--idle-timeout-ms must be more than 0");
                }
                idle_timeout = Duration::from_millis(millis);
            }
            other => bail!("unknown flag {other}"),
        }
    }
    Ok(Args {
        port: port.context("--port is required")?,
        cert_file: cert_file.context("--cert-file is required")?,
        key_file: key_file.context("--key-file is required")?,
        idle_timeout,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    let log = Log::stdout();
    let config = HubConfig {
        log: Some(log.clone()),
        ..HubConfig::default()
    };
    let limits = quic_limits(&config.streams, args.idle_timeout);
    let server = WtServer::bind(args.port, &args.cert_file, &args.key_file, limits)?;
    println!("LISTENING {} port={}", config.implementation, args.port);
    let (hub, hub_task) = hub::spawn(config);
    tokio::select! {
        () = accept_sessions(&server, &hub, &log) => bail!("the server stopped accepting"),
        // WHY end the process: the hub is the one task that serves every
        // session. Without it the relay would go on accepting connections that
        // nobody answers.
        ended = hub_task => bail!("the relay's hub ended: {ended:?}"),
    }
}

async fn accept_sessions(server: &WtServer, hub: &HubHandle<WtConnection>, log: &Log) {
    let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    loop {
        let Ok(permit) = Arc::clone(&handshakes).acquire_owned().await else {
            return;
        };
        let Some(incoming) = server.incoming().await else {
            return;
        };
        let hub = hub.clone();
        let log = log.clone();
        // Each handshake runs on its own, so that a slow client does not hold
        // up the others.
        tokio::spawn(async move {
            let accepted = tokio::time::timeout(HANDSHAKE_TIMEOUT, accept(incoming)).await;
            drop(permit);
            match accepted {
                Ok(Ok((connection, asked))) => {
                    log.print(format!(
                        "SESSION_ACCEPTED authority={} path={}",
                        asked.authority, asked.path
                    ));
                    hub.add_session(connection).await;
                }
                Ok(Err(error)) => log.print(format!("HANDSHAKE_FAILED {error}")),
                Err(_) => log.print("HANDSHAKE_FAILED timed out".to_string()),
            }
        });
    }
}
