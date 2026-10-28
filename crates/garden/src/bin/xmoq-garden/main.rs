//! The garden of a room (`crates/garden/README.md`): a client of the relay
//! that follows what the actors of every room do, changes each room's
//! garden, and publishes it. It goes on until it is told to stop (Ctrl-C,
//! SIGTERM), and connects again after a session ends.
//!
//! What it prints to stdout, one line for each thing it does, is what an
//! interop scene waits for.

mod session;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use xmoq_again::next_session::{Ended, NextSession, Pause};
use xmoq_garden::rooms::Rooms;
use xmoq_moqt_io::wt;
use xmoq_moqt_io::{StreamConfig, client};

use crate::session::SessionEnd;

async fn run(args: xmoq_garden::args::Args) -> Result<()> {
    let mut stop = client::on_signals(client::Signals::SERVING);
    let mut rooms = Rooms::default();
    let streams = StreamConfig::default();
    // As the chat's gatherer: the New Session URI of a GOAWAY at once, a
    // wait that grows after any other end, and no end ends the client.
    let mut next = NextSession::new(Pause::SERVING, true, false);
    let started = Instant::now();
    let now_ms = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let key = args.auth_key.as_deref().map(xmoq_room_token::RoomKey::new);
    let mut url = next.next(&args.url, now_ms()).url;
    loop {
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
                let result = session::tend(
                    &connection,
                    &mut kept,
                    &mut rooms,
                    key.as_ref(),
                    args.ask_again,
                    &mut stop,
                )
                .await;
                match &result {
                    Ok(ended) => println!("SESSION_ENDED {ended:?}"),
                    Err(error) => println!("SESSION_FAILED {error:#}"),
                }
                client::close(connection.as_ref(), &result, client::CloseWait::SERVING).await;
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
        next.ended(ended);
        let after = next.next(&args.url, now_ms());
        url = after.url;
        let wait = Duration::from_millis(after.wait_ms);
        println!("CONNECTING_AGAIN url={url} in_ms={}", wait.as_millis());
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            _ = stop.changed() => return Ok(()),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match xmoq_garden::args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n\n{}", xmoq_garden::args::USAGE);
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
