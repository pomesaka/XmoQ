//! The transcriber (`crates/transcriber/README.md`): a client of the relay
//! that listens to the audio tracks of the broadcasts announced under its
//! prefixes, reads their words (whisper.cpp by default, `--engine`), and publishes them. It goes
//! on until it is told to stop (Ctrl-C, SIGTERM), and connects again after
//! a session ends.
//!
//! What it prints to stdout, one line for each thing it does, is what the
//! interop driver (`interop/transcriber.ts`) waits for.

mod reader;
mod session;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use tokio::sync::watch;
use xmoq_again::next_session::Ended as SessionEnded;
use xmoq_moqt_io::wt;
use xmoq_moqt_io::{Connection, StreamConfig, client};
use xmoq_speech::engine::EngineOptions;
use xmoq_speech::model::{DEFAULT_VAD_MODEL, default_model_dir, model_file};
use xmoq_speech::vad::Silero;
use xmoq_track_serving::backoff::NextTry;
use xmoq_transcriber::args::Args;
use xmoq_transcriber::transcript::Transcript;

use crate::reader::{Reader, wall_ms};
use crate::session::Ended;

/// How long a closed session gets to reach the relay before the transcriber
/// goes on, as the gatherer gives it.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// The directory the models are fetched into, where it is known.
fn cache_dir() -> Option<PathBuf> {
    default_model_dir(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}

/// The file a flag gives, or the model `name` in the cache.
fn model_path(given: Option<&PathBuf>, name: &str) -> Result<PathBuf> {
    if let Some(model) = given {
        return Ok(model.clone());
    }
    let dir =
        cache_dir().context("neither XDG_CACHE_HOME nor HOME is set: give the model's file")?;
    Ok(model_file(&dir, name))
}

async fn run(args: Args) -> Result<()> {
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
    // Each broadcast loads one of its own; loaded once here, a model that is
    // missing stops the transcriber at the start.
    let vad = if args.vad {
        let vad = model_path(args.vad_model.as_ref(), DEFAULT_VAD_MODEL)?;
        Silero::open(&vad).context("mise run speech-model -- silero-v5.1.2 fetches it")?;
        println!("VAD_LOADED path={}", vad.display());
        Some(vad)
    } else {
        None
    };
    let loading = Instant::now();
    // `scratch` is held until `run` returns, on SIGTERM as well, and the
    // readers' programs and directory go with it.
    let xmoq_speech::engine::Opened {
        readers,
        said,
        scratch: _held_until_the_end,
    } = xmoq_speech::engine::open(
        args.engine,
        &EngineOptions {
            model: args.model.clone(),
            cache: cache_dir(),
            language: args.language.clone(),
            command: args.command.clone(),
            command_limits: args.command_limits,
        },
        args.readers,
    )?;
    println!("MODEL_LOADED {} ms={}", said, loading.elapsed().as_millis());
    let (mut reader, mut read) = Reader::start(readers, args.waiting);
    let first = wall_ms();
    let mut transcripts: Vec<Transcript> = args
        .prefixes
        .iter()
        .map(|_| Transcript::new(first, args.keep))
        .collect();
    let streams = StreamConfig::default();
    // Where and when the next session goes (`NextTry`): the New Session URI
    // of a GOAWAY, at once; after any other end, after a wait that grows.
    let mut next = NextTry::default();
    let key = args.auth_key.as_deref().map(xmoq_room_token::RoomKey::new);
    let mut url = next.first(&args.url);
    loop {
        let began = Instant::now();
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
                let result = session::transcribe(
                    &connection,
                    &mut kept,
                    &args,
                    (&mut reader, &mut transcripts, key.as_ref(), vad.as_deref()),
                    &mut read,
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
                    Ok(Ended::Stopped) => return Ok(()),
                    Ok(Ended::Moved(to)) => SessionEnded::Moved(to),
                    Err(_) => SessionEnded::Failed { began: true },
                }
            }
            Err(error) => {
                println!("NOT_CONNECTED {error}");
                SessionEnded::Failed { began: false }
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
    let args = match xmoq_transcriber::args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n\n{}", xmoq_transcriber::args::USAGE);
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
