//! The gatherer of the chat (`crates/chat/README.md`): a client of the relay
//! that takes the lines of every room, numbers them, keeps them in a SQLite
//! file and publishes them. It goes on until it is told to stop (Ctrl-C,
//! SIGTERM), and connects again after a session ends.
//!
//! What it prints to stdout, one line for each thing it does, is what the
//! interop driver (`interop/chat.ts`) waits for.

mod session;
mod storage;

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::watch;
use xmoq_chat::gather::Gatherer;
use xmoq_chat::rooms::MAX_ROOMS;
use xmoq_moqt_io::wt;
use xmoq_moqt_io::{Connection, StreamConfig};
use xmoq_track_serving::backoff::Backoff;

use crate::session::{Ended, NoSetupInTime};
use crate::storage::Store;

/// How long a closed session gets to reach the relay before the gatherer
/// goes on, as the Rust client gives it (`CLOSE_GRACE`).
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// §3.5: "`CONTROL_MESSAGE_TIMEOUT` (0x11): The session was closed because
/// the peer took too long to respond to a control message", for the SETUP.
const CONTROL_MESSAGE_TIMEOUT: u32 = 0x11;

/// The code a session that failed is closed with (§3.5): the one the
/// protocol error asks for, or `NO_ERROR` for failures that are not the
/// relay's.
fn termination_code(result: &Result<Ended>) -> u32 {
    let Err(error) = result else { return 0 };
    if error.is::<NoSetupInTime>() {
        return CONTROL_MESSAGE_TIMEOUT;
    }
    // WHY the chain: `context` wraps the cause.
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<xmoq_moqt::Error>())
        .and_then(xmoq_moqt::Error::termination_code)
        .map_or(0, xmoq_moqt::TerminationCode::code)
}

async fn run(args: xmoq_chat::args::Args) -> Result<()> {
    let (stopping, mut stop) = watch::channel(false);
    tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let Ok(mut terminate) = signal(SignalKind::terminate()) else {
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        let _ = stopping.send(true);
    });
    let store = Store::open(Path::new(&args.store))?;
    let mut gatherer = Gatherer::new(args.limits);
    println!(
        "STORE path={:?} rooms={}",
        args.store,
        store.rooms(MAX_ROOMS)?.len()
    );
    let streams = StreamConfig::default();
    // WHY a wait that grows (`xmoq_track_serving::backoff`): a relay that ends every
    // session at once would have the gatherer connect every second for
    // good.
    let mut backoff = Backoff::default();
    let key = args.auth_key.as_deref().map(xmoq_room_token::RoomKey::new);
    loop {
        let began = std::time::Instant::now();
        let connecting = wt::connect(&args.url, args.cert_sha256, &streams);
        let connection = tokio::select! {
            connected = connecting => connected,
            _ = stop.changed() => return Ok(()),
        };
        match connection {
            Ok(connection) => {
                let connection = Arc::new(connection);
                println!("CONNECTED url={}", args.url);
                let result = session::gather(
                    &connection,
                    &store,
                    (&mut gatherer, &args.rooms, key.as_ref()),
                    args.ask_again,
                    &mut stop,
                )
                .await;
                match &result {
                    Ok(ended) => println!("SESSION_ENDED {ended:?}"),
                    Err(error) => println!("SESSION_FAILED {error:#}"),
                }
                connection.close(termination_code(&result), b"");
                let _ = tokio::time::timeout(CLOSE_GRACE, connection.closed()).await;
                if matches!(result, Ok(Ended::Stopped)) {
                    return Ok(());
                }
            }
            Err(error) => println!("NOT_CONNECTED {error}"),
        }
        let wait = backoff.after(began.elapsed());
        println!("CONNECTING_AGAIN in_ms={}", wait.as_millis());
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            _ = stop.changed() => return Ok(()),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match xmoq_chat::args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n\n{}", xmoq_chat::args::USAGE);
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
