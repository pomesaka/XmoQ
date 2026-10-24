//! The transcriber (`crates/transcriber/README.md`): a client of the relay
//! that listens to the audio tracks of the broadcasts announced under its
//! prefixes, reads their words with whisper.cpp, and publishes them. It goes
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
use xmoq_moqt_io::wt;
use xmoq_moqt_io::{Connection, StreamConfig};
use xmoq_speech::model::{DEFAULT_MODEL, default_model_dir, model_file};
use xmoq_speech::whisper::Whisper;
use xmoq_track_serving::backoff::Backoff;
use xmoq_transcriber::args::Args;
use xmoq_transcriber::transcript::Transcript;

use crate::reader::{Reader, wall_ms};
use crate::session::{Ended, NoSetupInTime};

/// How long a closed session gets to reach the relay before the transcriber
/// goes on, as the gatherer gives it.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// §3.5: "`CONTROL_MESSAGE_TIMEOUT` (0x11)", for the SETUP.
const CONTROL_MESSAGE_TIMEOUT: u32 = 0x11;

/// The code a session that failed is closed with (§3.5), as the gatherer
/// closes it.
fn termination_code(result: &Result<Ended>) -> u32 {
    let Err(error) = result else { return 0 };
    if error.is::<NoSetupInTime>() {
        return CONTROL_MESSAGE_TIMEOUT;
    }
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<xmoq_moqt::Error>())
        .and_then(xmoq_moqt::Error::termination_code)
        .map_or(0, xmoq_moqt::TerminationCode::code)
}

/// `--model`, or the default model in the cache.
fn model(args: &Args) -> Result<PathBuf> {
    if let Some(model) = &args.model {
        return Ok(model.clone());
    }
    let dir = default_model_dir(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
        .context("neither XDG_CACHE_HOME nor HOME is set: give --model")?;
    Ok(model_file(&dir, DEFAULT_MODEL))
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
    let model = model(&args)?;
    let loading = Instant::now();
    let whisper = Whisper::open(&model, &args.language)
        .context("mise run speech-model fetches the default model")?;
    println!(
        "MODEL_LOADED path={} language={} ms={}",
        model.display(),
        args.language,
        loading.elapsed().as_millis()
    );
    let (mut reader, mut read) = Reader::start(whisper, args.waiting);
    let first = wall_ms();
    let mut transcripts: Vec<Transcript> = args
        .prefixes
        .iter()
        .map(|_| Transcript::new(first, args.keep))
        .collect();
    let streams = StreamConfig::default();
    let mut backoff = Backoff::default();
    let key = args.auth_key.as_deref().map(xmoq_room_token::RoomKey::new);
    loop {
        let began = Instant::now();
        let connecting = wt::connect(&args.url, args.cert_sha256, &streams);
        let connection = tokio::select! {
            connected = connecting => connected,
            _ = stop.changed() => return Ok(()),
        };
        match connection {
            Ok(connection) => {
                let connection = Arc::new(connection);
                println!("CONNECTED url={}", args.url);
                let result = session::transcribe(
                    &connection,
                    &args,
                    (&mut reader, &mut transcripts, key.as_ref()),
                    &mut read,
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
