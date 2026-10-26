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
use xmoq_again::next_session::Ended;
use xmoq_chat::gather::Gatherer;
use xmoq_chat::rooms::MAX_ROOMS;
use xmoq_moqt_io::wt;
use xmoq_moqt_io::{Connection, StreamConfig, client};
use xmoq_track_serving::backoff::NextTry;

use crate::session::SessionEnd;
use crate::storage::Store;

/// How long a closed session gets to reach the relay before the gatherer
/// goes on, as the Rust client gives it (`CLOSE_GRACE`).
const CLOSE_GRACE: Duration = Duration::from_secs(2);

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
    // Where and when the next session goes (`NextTry`): the New Session URI
    // of a GOAWAY, at once; after any other end, after a wait that grows.
    let mut next = NextTry::default();
    let key = args.auth_key.as_deref().map(xmoq_room_token::RoomKey::new);
    let mut url = next.first(&args.url);
    loop {
        let began = std::time::Instant::now();
        let connecting = wt::connect(&url, args.cert_sha256, &streams);
        let connection = tokio::select! {
            connected = connecting => connected,
            _ = stop.changed() => return Ok(()),
        };
        let ended = match connection {
            Ok(connection) => {
                let connection = Arc::new(connection);
                println!("CONNECTED url={url}");
                // Dropped once the session is closed.
                let mut kept = client::UntilClosed::default();
                let result = session::gather(
                    &connection,
                    &mut kept,
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
                connection.close(client::termination_code_of(&result), b"");
                let _ = tokio::time::timeout(CLOSE_GRACE, connection.closed()).await;
                match result {
                    Ok(SessionEnd::Stopped) => return Ok(()),
                    Ok(SessionEnd::Moved(to)) => Ended::Moved(to),
                    Err(_) => Ended::Failed { began: true },
                }
            }
            Err(error) => {
                println!("NOT_CONNECTED {error}");
                Ended::Failed { began: false }
            }
        };
        let wait;
        (url, wait) = next.after(&args.url, ended, began.elapsed());
        println!("CONNECTING_AGAIN url={url} in_ms={}", wait.as_millis());
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
