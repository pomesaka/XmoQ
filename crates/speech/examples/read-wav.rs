//! Reads a WAV file as if it came on an audio track, and says how long each
//! piece took: for measuring the models (README), not for use.
//!
//! ```sh
//! afconvert -f WAVE -d LEI16@48000 -c 1 voice.aiff voice.wav   # macOS; `say -o voice.aiff ...`
//! cargo run --release --example read-wav -p xmoq-speech -- <model file> <language> voice.wav [<model of voice activity>]
//! ```
//!
//! The file is one channel of 16-bit samples at 48 kHz. It is encoded to
//! packets of Opus of 20 ms, as a browser sends them, and the packets are
//! given to a [`Listener`] one after another, and each piece is read as
//! soon as it is cut. A voice is told by the model of voice activity where
//! its file is given, and by loudness where it is not.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail};
use xmoq_speech::cut::{Cut, Piece};
use xmoq_speech::engine::{Engine, EngineOptions};
use xmoq_speech::judge::{Judge, ModelJudge, VOICE_DBFS};
use xmoq_speech::vad::Silero;
use xmoq_speech::{AudioPacket, Listener, Words};

/// `N` bytes of `bytes` from `at`.
fn bytes_at<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N]> {
    Ok(bytes
        .get(at..at + N)
        .context("a chunk too short")?
        .try_into()?)
}

/// The samples of a WAV file of one channel of 16 bits at 48 kHz.
fn samples(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        bail!("not a WAV file");
    }
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id: [u8; 4] = bytes_at(bytes, at)?;
        let size = usize::try_from(u32::from_le_bytes(bytes_at(bytes, at + 4)?))?;
        let body = bytes
            .get(at + 8..at + 8 + size)
            .context("a chunk past the end")?;
        match &id {
            b"fmt " => {
                let channels = u16::from_le_bytes(bytes_at(body, 2)?);
                let rate = u32::from_le_bytes(bytes_at(body, 4)?);
                let bits = u16::from_le_bytes(bytes_at(body, 14)?);
                if (channels, rate, bits) != (1, 48_000, 16) {
                    bail!("{channels} channels, {rate} Hz, {bits} bits; not 1, 48000, 16");
                }
            }
            b"data" => {
                return Ok(body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
                    .collect());
            }
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    Err(anyhow!("no data chunk"))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (model, language, wav, vad) = match args.as_slice() {
        [model, language, wav] => (model, language, wav, None),
        [model, language, wav, vad] => (model, language, wav, Some(vad)),
        _ => bail!("read-wav <model file> <language> <wav file> [<model of voice activity>]"),
    };
    let pcm = samples(&std::fs::read(wav)?)?;
    let loading = Instant::now();
    let options = EngineOptions {
        model: Some(model.into()),
        cache: None,
        language: language.clone(),
        command: Vec::new(),
        command_limits: xmoq_speech::command::CommandLimits::default(),
    };
    let mut readers = xmoq_speech::engine::open(Engine::Whisper, &options, 1)?.readers;
    let whisper = readers.first_mut().context("a reader")?;
    let judge = match vad {
        Some(vad) => Judge::Model(ModelJudge::new(
            Box::new(Silero::open(Path::new(vad))?),
            0.5,
        )),
        None => Judge::Loudness(VOICE_DBFS),
    };
    let mut listener = Listener::new(judge, Cut::default())?;
    println!("loaded in {} ms", loading.elapsed().as_millis());
    let mut encoder = opus_rs::OpusEncoder::new(48_000, 1, opus_rs::Application::Voip)
        .map_err(|error| anyhow!("{error}"))?;
    let mut packet = vec![0; 1500];
    let mut timestamp_us = 0;
    let began = Instant::now();
    for frame in pcm.chunks(960) {
        let mut frame = frame.to_vec();
        frame.resize(960, 0.0);
        let len = encoder
            .encode(&frame, 960, &mut packet)
            .map_err(|error| anyhow!("{error}"))?;
        let pushed = Instant::now();
        let opus = packet.get(..len).context("encoder")?;
        for piece in listener.push(AudioPacket { timestamp_us, opus }) {
            say(whisper.as_mut(), &piece, pushed)?;
        }
        timestamp_us += 20_000;
    }
    let ended = Instant::now();
    for piece in listener.end() {
        say(whisper.as_mut(), &piece, ended)?;
    }
    println!(
        "{} ms of sound in {} ms; {:?}",
        timestamp_us / 1000,
        began.elapsed().as_millis(),
        listener.left()
    );
    Ok(())
}

fn say(whisper: &mut dyn Words, piece: &Piece, pushed: Instant) -> Result<()> {
    let text = whisper.text(&piece.samples)?;
    let seconds = |us: u64| f64::from(u32::try_from(us / 1000).unwrap_or(u32::MAX)) / 1000.0;
    println!(
        "{:6.2}-{:6.2} s, read in {:5} ms: {}",
        seconds(piece.start_us),
        seconds(piece.end_us),
        pushed.elapsed().as_millis(),
        text
    );
    Ok(())
}
