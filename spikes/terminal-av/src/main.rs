//! One viewer for the terminal: subscribes to a video track and an audio
//! track on one session, and plays both by the Timestamps of LOC: the
//! picture with the Kitty graphics protocol, the sound on the machine's
//! output.
//!
//!   xmoq-watch --url https://127.0.0.1:4443/ --cert-sha256 <hex> [--namespace xmoq/demo]
//!       [--video video] [--audio audio] [--buffer-ms 50] [--volume 1] [--scale 1]
//!       [--medium file|direct] [--out <file>] [--seconds N]
//!
//! The clock: the smallest (arrival - Timestamp) seen on either track is
//! taken for the time an Object needs at least, and every Object is due
//! `--buffer-ms` after its Timestamp plus that. Both tracks have to carry
//! Timestamps of one clock, as the tracks of the demo do.
//!
//! It ends with Ctrl-C, after `--seconds`, or when a track ends, and prints
//! `AV_PLAYED` to stderr.

mod picture;
mod screen;
mod sound;

use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::{ObjectStatus, SubgroupObject};
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::message::setup_option;
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{RequestEvent, RequestId, Role, Session, Transport};
use xmoq_moqt_io::wt::{self, WtConnection};
use xmoq_moqt_io::{
    Connection, RequestReader, RequestStreamEvent, SendStream, StreamConfig, StreamStops, Trace, UniEvent, read_uni_streams,
    send_message,
};

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

struct Args {
    url: String,
    cert_sha256: [u8; 32],
    namespace: String,
    video: String,
    audio: String,
    buffer: Duration,
    volume: f32,
    scale: usize,
    medium: screen::Medium,
    out: Option<String>,
    seconds: Option<u64>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        url: String::new(),
        cert_sha256: [0; 32],
        namespace: "xmoq/demo".to_string(),
        video: "video".to_string(),
        audio: "audio".to_string(),
        buffer: Duration::from_millis(50),
        volume: 1.0,
        scale: 1,
        medium: screen::Medium::File,
        out: None,
        seconds: None,
    };
    let mut given = std::env::args().skip(1);
    while let Some(flag) = given.next() {
        let value = given.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--url" => args.url = value,
            "--cert-sha256" => {
                if value.len() != 64 {
                    bail!("--cert-sha256 is 64 hex digits");
                }
                for (at, byte) in args.cert_sha256.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&value[at * 2..at * 2 + 2], 16).context("--cert-sha256")?;
                }
            }
            "--namespace" => args.namespace = value,
            "--video" => args.video = value,
            "--audio" => args.audio = value,
            "--buffer-ms" => args.buffer = Duration::from_millis(value.parse().context("--buffer-ms")?),
            "--volume" => args.volume = value.parse().context("--volume")?,
            "--scale" => args.scale = value.parse().context("--scale")?,
            "--medium" => {
                args.medium = match value.as_str() {
                    "file" => screen::Medium::File,
                    "direct" => screen::Medium::Direct,
                    other => bail!("--medium {other}: file or direct"),
                }
            }
            "--out" => args.out = Some(value),
            "--seconds" => args.seconds = Some(value.parse().context("--seconds")?),
            other => bail!("unknown flag {other}"),
        }
    }
    if args.url.is_empty() {
        bail!("--url is required");
    }
    Ok(args)
}

/// A subscription: its request stream, kept open for as long as the track is wanted.
struct Subscription {
    request: RequestId,
    track_alias: u64,
    reader: RequestReader<Recv>,
    _send: Send,
}

async fn subscribe(
    connection: &WtConnection,
    session: &mut Session,
    namespace: &TrackNamespace,
    name: &str,
) -> Result<Subscription> {
    let track = FullTrackName::new(namespace.clone(), name.as_bytes().to_vec())?;
    let parameters = vec![
        Parameter::Forward(true),
        Parameter::SubscriberPriority(128),
        Parameter::GroupOrder(GroupOrder::Ascending),
    ];
    let (request, message) = session.subscribe(track, parameters)?;
    let (mut send, recv) = connection.open_bi().await?;
    let trace = Trace::default();
    send_message(&mut send, &message, &trace, name).await?;
    let mut reader = RequestReader::new(recv, &trace, name);
    let RequestStreamEvent::Message(response) = reader.next_event().await? else {
        bail!("the relay closed the SUBSCRIBE of {name} without an answer");
    };
    match session.on_request_message(request, response)? {
        RequestEvent::SubscribeEstablished { track_alias, .. } => Ok(Subscription { request, track_alias, reader, _send: send }),
        other => bail!("SUBSCRIBE of {name} was refused: {other:?}"),
    }
}

/// The Timestamp of LOC of an Object, in microseconds.
fn timestamp_of(object: &SubgroupObject) -> Option<u64> {
    let timing = loc::read_timing(object.properties.iter().map(|pair: &KeyValuePair| {
        let value = match pair.value() {
            Value::Varint(value) => Some(*value),
            Value::Bytes(_) => None,
        };
        (pair.ty(), value)
    }))
    .ok()??;
    // A Timescale of its own is not handled: the demo has none (wall-clock microseconds).
    timing.timescale.is_none().then_some(timing.timestamp)
}

/// When Objects are due, from their Timestamps.
struct Clock {
    began: Instant,
    /// The smallest (arrival - Timestamp), in microseconds of `began`.
    least: Option<i128>,
    buffer: Duration,
}

impl Clock {
    fn due(&mut self, timestamp: u64) -> Instant {
        let arrived = i128::try_from(self.began.elapsed().as_micros()).unwrap_or(0);
        let took = arrived - i128::from(timestamp);
        let least = self.least.map_or(took, |least| least.min(took));
        self.least = Some(least);
        let at = u64::try_from((i128::from(timestamp) + least).max(0)).unwrap_or(0);
        self.began + Duration::from_micros(at) + self.buffer
    }
}

fn percentile(sorted: &[f32], share: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() as f32 * share) as usize).min(sorted.len() - 1)]
}

fn spread(mut values: Vec<f32>) -> String {
    values.sort_by(f32::total_cmp);
    format!(
        "{:.1}/{:.1}/{:.1}/{:.1}",
        values.first().copied().unwrap_or(0.0),
        percentile(&values, 0.5),
        percentile(&values, 0.95),
        values.last().copied().unwrap_or(0.0)
    )
}

async fn run(args: &Args) -> Result<()> {
    let connection = Arc::new(wt::connect(&args.url, args.cert_sha256, &StreamConfig::default()).await?);
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let stops = StreamStops::default();
    let (queue, mut events) = mpsc::channel(64);
    let _streams = read_uni_streams(Arc::clone(&connection), StreamConfig::default(), stops.clone(), queue, |event| event);
    // §3.3: each peer opens one control stream that begins with SETUP. It stays open.
    let mut control = connection.open_uni().await?;
    let setup = session.setup(vec![KeyValuePair::bytes(setup_option::MOQT_IMPLEMENTATION, b"xmoq-watch (spike)".to_vec())?])?;
    send_message(&mut control, &setup, &Trace::default(), "control").await?;
    match events.recv().await {
        Some(UniEvent::Control(message)) => session.on_control_message(message)?,
        other => bail!("no SETUP from the relay: {other:?}"),
    };

    let namespace = TrackNamespace::new(args.namespace.split('/').map(|field| field.as_bytes().to_vec()).collect())?;
    let mut video = subscribe(&connection, &mut session, &namespace, &args.video).await?;
    let mut audio = subscribe(&connection, &mut session, &namespace, &args.audio).await?;

    let to: Box<dyn std::io::Write + std::marker::Send> = match &args.out {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout()),
    };
    let screen = screen::Screen::new(to, args.medium, args.scale);
    let (frames, for_picture) = std_mpsc::channel();
    let picture = std::thread::spawn(move || picture::run(for_picture, screen));
    let mut sound = sound::Sound::open(args.volume)?;

    let began = Instant::now();
    let mut clock = Clock { began, least: None, buffer: args.buffer };
    let end = async {
        match args.seconds {
            Some(seconds) => tokio::time::sleep(Duration::from_secs(seconds)).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(end);
    let (mut no_timestamp, mut streams_reset) = (0u64, 0u64);
    let why = loop {
        tokio::select! {
            () = &mut end => break "the time was over",
            _ = tokio::signal::ctrl_c() => break "interrupted",
            event = video.reader.next_event() => {
                if let RequestStreamEvent::Message(message) = event? {
                    session.on_request_message(video.request, message)?;
                }
                break "the video track ended";
            }
            event = audio.reader.next_event() => {
                if let RequestStreamEvent::Message(message) = event? {
                    session.on_request_message(audio.request, message)?;
                }
                break "the audio track ended";
            }
            event = events.recv() => match event.ok_or_else(|| anyhow!("the connection is gone"))? {
                UniEvent::Object { header, object, .. } => {
                    if object.status != ObjectStatus::Normal {
                        continue;
                    }
                    let Some(timestamp) = timestamp_of(&object) else {
                        no_timestamp += 1;
                        continue;
                    };
                    let due = clock.due(timestamp);
                    if header.track_alias == video.track_alias {
                        let annex_b = loc::to_annex_b(&object.payload).map_err(|error| anyhow!("a video payload: {}", error.0))?;
                        let _ = frames.send(picture::Frame { group: header.group_id, object: object.object_id, annex_b, due });
                    } else if header.track_alias == audio.track_alias {
                        // WHY here and not on a thread of its own: 2.5 ms for a packet of 20 ms.
                        sound.take(&object.payload, due);
                    }
                }
                UniEvent::SubgroupClosed { header, end, .. } => {
                    session.on_data_stream_closed(header.track_alias);
                    if !matches!(end, xmoq_moqt_io::StreamEnd::Fin) {
                        streams_reset += 1;
                    }
                }
                UniEvent::SubgroupOpened { .. } => {}
                UniEvent::Failed(error) => return Err(error.into()),
                other => bail!("not expected of the relay: {other:?}"),
            },
        }
    };
    let seconds = began.elapsed().as_secs_f64();
    drop(frames);
    let seen = picture.join().map_err(|_| anyhow!("the picture's thread panicked"))?;
    let heard = sound.counts();
    connection.close(0, b"");
    let _ = tokio::time::timeout(Duration::from_millis(500), connection.closed()).await;
    control.finish();

    // The square and the beep begin at the same moment of the publisher's clock: how far apart
    // they left this program. More than zero: the picture is the later one.
    let apart: Vec<f32> = seen
        .marks
        .iter()
        .filter_map(|drawn| {
            heard
                .marks
                .iter()
                .map(|played| if drawn >= played { (*drawn - *played).as_secs_f32() } else { -(*played - *drawn).as_secs_f32() } * 1000.0)
                .min_by(|a, b| a.abs().total_cmp(&b.abs()))
        })
        .filter(|ms| ms.abs() < 1000.0)
        .collect();
    let shown_for = match (seen.first, seen.last) {
        (Some(first), Some(last)) => (last - first).as_secs_f64(),
        _ => 0.0,
    };
    let rms = if heard.given == 0 { 0.0 } else { (heard.squares / heard.given as f64).sqrt() };
    eprintln!(
        "AV_PLAYED why=\"{why}\" seconds={seconds:.1} buffer_ms={} volume={} \
         video_came={} video_left_out={} video_drawn={} fps={:.1} video_off_ms_min/p50/p95/max={} decode_ms_a_frame={:.2} draw_ms_a_frame={:.2} \
         audio_packets={} audio_refused={} audio_seconds={:.2} underruns={} silence_waiting_ms={} audio_left_out_late={} audio_cut_late={} audio_dropped_full={} \
         audio_off_ms_min/p50/p95/max={} peak={:.3} rms={:.3} \
         marks_video={} marks_audio={} picture_after_sound_ms_min/p50/p95/max={} no_timestamp={no_timestamp} streams_reset={streams_reset}",
        args.buffer.as_millis(),
        args.volume,
        seen.came,
        seen.left_out,
        seen.drawn,
        if shown_for > 0.0 { (seen.drawn.saturating_sub(1)) as f64 / shown_for } else { 0.0 },
        spread(seen.off_ms),
        seen.decoding.as_secs_f64() * 1000.0 / seen.decoded.max(1) as f64,
        seen.drawing.as_secs_f64() * 1000.0 / seen.drawn.max(1) as f64,
        heard.packets,
        heard.refused,
        heard.given as f64 / 48_000.0,
        heard.underruns,
        heard.waits / 48,
        heard.left_out_late,
        heard.cut_late,
        heard.dropped_full,
        spread(heard.off_ms),
        heard.peak,
        rms,
        seen.marks.len(),
        heard.marks.len(),
        spread(apart),
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    run(&args).await
}
