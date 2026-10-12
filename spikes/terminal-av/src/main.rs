//! One viewer for the terminal: subscribes to a video track and an audio
//! track on one session, and plays both by the Timestamps of LOC: the
//! picture with the Kitty graphics protocol, the sound on the machine's
//! output.
//!
//!   xmoq-watch --url https://127.0.0.1:4443/ --cert-sha256 <hex> [--namespace xmoq/demo]
//!       [--video <track> --audio <track>] [--buffer-ms 50] [--volume 1] [--scale 1]
//!       [--medium file|direct] [--out <file>] [--seconds N]
//!   xmoq-watch --room http://127.0.0.1:8787/rooms/demo [--name terminal] [--say "a line"] [--token no]
//!
//! With `--room` the room says where the broadcast is (the relay, the hash
//! of its certificate, the namespace) and gives a token for the relay,
//! which goes into SETUP and into both SUBSCRIBEs; `--token no` asks for
//! none. The chat of the room is written under the picture, and to stderr
//! where stderr is no terminal (`CHAT ...`). `--say` sends one line. When
//! the room's broadcast begins, ends or moves, the viewer follows it.
//!
//! Keys, where stdin is a terminal: `q` or Ctrl-C ends, `+` and `-` change
//! the volume, `m` mutes, `t` or Enter begins a line for the chat (Enter
//! sends it, Esc gives it up). `--log <file>` takes the lines about the room
//! and the keys, for a run whose stderr is the terminal.
//!
//! The tracks are found in the MSF catalog of the namespace (the track
//! `catalog`): the first video track of H.264 and the first audio track of
//! Opus, both with LOC packaging. `--video` and `--audio` name them
//! instead, and no catalog is asked for.
//!
//! The clock: the smallest (arrival - Timestamp) seen on either track is
//! taken for the time an Object needs at least, and every Object is due
//! `--buffer-ms` after its Timestamp plus that. Both tracks have to carry
//! Timestamps of one clock, as the tracks of the demo do.
//!
//! It ends with Ctrl-C, after `--seconds`, or when a track ends, and prints
//! `AV_PLAYED` to stderr.

mod catalog;
mod keys;
mod picture;
mod room;
mod screen;
mod sound;

use std::io::{IsTerminal, Write};
use std::sync::{Arc, Mutex};
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
    Connection, RequestReader, RequestStreamEvent, StreamConfig, StreamStops, Trace, UniEvent, read_uni_streams,
    send_message,
};

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

struct Args {
    url: String,
    cert_sha256: [u8; 32],
    namespace: String,
    /// The names of the tracks; None: what the catalog has.
    video: Option<String>,
    audio: Option<String>,
    buffer: Duration,
    volume: f32,
    scale: usize,
    medium: screen::Medium,
    out: Option<String>,
    seconds: Option<u64>,
    room: Option<String>,
    name: String,
    say: Option<String>,
    ask_token: bool,
    log: Option<String>,
}

fn hash_of(hex: &str) -> Result<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        bail!("the hash of a certificate is 64 hex digits");
    }
    let mut hash = [0; 32];
    for (at, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[at * 2..at * 2 + 2], 16).context("the hash of a certificate")?;
    }
    Ok(hash)
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        url: String::new(),
        cert_sha256: [0; 32],
        namespace: "xmoq/demo".to_string(),
        video: None,
        audio: None,
        buffer: Duration::from_millis(50),
        volume: 1.0,
        scale: 1,
        medium: screen::Medium::File,
        out: None,
        seconds: None,
        room: None,
        name: "terminal".to_string(),
        say: None,
        ask_token: true,
        log: None,
    };
    let mut given = std::env::args().skip(1);
    while let Some(flag) = given.next() {
        let value = given.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--url" => args.url = value,
            "--cert-sha256" => {
                args.cert_sha256 = hash_of(&value)?;
            }
            "--namespace" => args.namespace = value,
            "--video" => args.video = Some(value),
            "--audio" => args.audio = Some(value),
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
            "--room" => args.room = Some(value),
            "--name" => args.name = value,
            "--say" => args.say = Some(value),
            "--token" => args.ask_token = value != "no",
            "--log" => args.log = Some(value),
            other => bail!("unknown flag {other}"),
        }
    }
    if args.url.is_empty() && args.room.is_none() {
        bail!("--url or --room is required");
    }
    Ok(args)
}

/// A subscription: its request stream, kept open for as long as the track is wanted.
pub(crate) struct Subscription {
    pub(crate) request: RequestId,
    pub(crate) track_alias: u64,
    reader: RequestReader<Recv>,
    _send: Send,
}

pub(crate) async fn subscribe(
    connection: &WtConnection,
    session: &mut Session,
    namespace: &TrackNamespace,
    name: &str,
    token: Option<&[u8]>,
) -> Result<Subscription> {
    let track = FullTrackName::new(namespace.clone(), name.as_bytes().to_vec())?;
    let mut parameters = vec![
        Parameter::Forward(true),
        Parameter::SubscriberPriority(128),
        Parameter::GroupOrder(GroupOrder::Ascending),
    ];
    if let Some(token) = token {
        parameters.push(Parameter::AuthorizationToken(token.to_vec()));
    }
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

/// What the viewer shares between its parts while it runs.
struct Ui {
    under: Arc<Mutex<screen::Under>>,
    volume: Arc<sound::Volume>,
    keys: keys::Keys,
    pressed: Option<mpsc::Receiver<u8>>,
    log: Option<std::fs::File>,
    /// Becomes true when the process is told to end.
    stop: tokio::sync::watch::Receiver<bool>,
    deadline: Option<tokio::time::Instant>,
}

impl Ui {
    /// A line about the room or the keys: to `--log`, or to stderr where that is no terminal.
    fn note(&mut self, line: &str) {
        if let Some(log) = &mut self.log {
            let _ = writeln!(log, "{line}");
        } else if !std::io::stderr().is_terminal() {
            eprintln!("{line}");
        }
    }

    fn chat(&mut self, line: String) {
        self.note(&format!("CHAT {line}"));
        let mut under = self.under.lock().expect("the lock");
        under.chat.push(line);
        under.version += 1;
    }

    /// Writes the line of the keys anew.
    fn show(&self) {
        let status = match self.keys.typing() {
            Some(line) => format!("> {line}_"),
            None => format!(
                "[q] quit  [+/-] volume {:.1}{}  [t] say",
                self.volume.level(),
                if self.volume.muted() { " (muted, [m])" } else { "  [m] mute" }
            ),
        };
        let mut under = self.under.lock().expect("the lock");
        under.status = status.chars().filter(|char| !char.is_control()).take(120).collect();
        under.version += 1;
    }

    /// The next thing that is not the broadcast's: a key that asks for something, the end of
    /// the time, or the order to end.
    async fn asked(&mut self) -> keys::Asked {
        loop {
            let pressed = async {
                match self.pressed.as_mut() {
                    Some(pressed) => pressed.recv().await,
                    None => std::future::pending().await,
                }
            };
            let deadline = async {
                match self.deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                () = deadline => return keys::Asked::Quit,
                _ = self.stop.changed() => return keys::Asked::Quit,
                byte = pressed => {
                    // Stdin is over: no more keys.
                    let Some(byte) = byte else {
                        self.pressed = None;
                        continue;
                    };
                    let asked = self.keys.press(byte);
                    match &asked {
                        Some(keys::Asked::Louder) => self.volume.change(0.1),
                        Some(keys::Asked::Quieter) => self.volume.change(-0.1),
                        Some(keys::Asked::Mute) => self.volume.mute(),
                        _ => {}
                    }
                    self.show();
                    match asked {
                        Some(keys::Asked::Louder | keys::Asked::Quieter | keys::Asked::Mute) => {
                            let line = format!("VOLUME level={:.1} muted={}", self.volume.level(), self.volume.muted());
                            self.note(&line);
                        }
                        Some(asked) => return asked,
                        None => {}
                    }
                }
            }
        }
    }
}

/// How a broadcast stopped being played.
enum Ended {
    /// The viewer is to end.
    Quit,
    /// The broadcast ended or moved: the room says what comes next.
    Again,
}

/// Waits until the room has a broadcast. None when the viewer is to end first.
async fn broadcast_of(room: &mut room::Room, ui: &mut Ui) -> Result<Option<room::Broadcast>> {
    loop {
        if let Some(broadcast) = room.current() {
            return Ok(Some(broadcast));
        }
        tokio::select! {
            said = room.next() => match said? {
                room::Said::Chat(line) => ui.chat(line),
                room::Said::Broadcast(_) => {}
            },
            asked = ui.asked() => match asked {
                keys::Asked::Say(text) => room.say(&text).await?,
                _ => return Ok(None),
            },
        }
    }
}

async fn run(args: &Args, ui: &mut Ui) -> Result<()> {
    // With a room: where the broadcast is, and a token, are the room's to say.
    let mut room = match &args.room {
        Some(address) => Some(room::Room::join(address, &args.name).await?),
        None => None,
    };
    if let (Some(room), Some(text)) = (&mut room, &args.say) {
        room.say(text).await?;
    }
    let mut first = true;
    loop {
        let (mut url, mut cert_sha256, mut namespace) = (args.url.clone(), args.cert_sha256, args.namespace.clone());
        let mut token = None;
        let mut playing = None;
        if let Some(room) = &mut room {
            if room.current().is_none() {
                ui.note("ROOM_BROADCAST none: waiting");
            }
            let Some(broadcast) = broadcast_of(room, ui).await? else { return Ok(()) };
            ui.note(&format!("ROOM_BROADCAST relay={} namespace={}", broadcast.relay_url, broadcast.namespace.join("/")));
            if args.ask_token {
                // §10.2.2: Alias Type USE_VALUE (0x3), Token Type 0, the value; as the pages do.
                token = room.token(&broadcast.namespace).await?.map(|text| [&[0x03, 0x00], text.as_bytes()].concat());
                ui.note(&format!("ROOM_TOKEN {}", if token.is_some() { "given" } else { "none: the room has no key" }));
            }
            url = broadcast.relay_url.clone();
            cert_sha256 = hash_of(&broadcast.cert_sha256_hex)?;
            namespace = broadcast.namespace.join("/");
            playing = Some(broadcast);
        }
        let mut began = false;
        let played = match wt::connect(&url, cert_sha256, &StreamConfig::default()).await {
            Ok(connection) => {
                let connection = Arc::new(connection);
                let played = play(args, &connection, &namespace, token.as_deref(), &mut room, playing.as_ref(), ui, &mut began).await;
                if played.is_err()
                    && let Ok(end) = tokio::time::timeout(Duration::from_millis(500), connection.closed()).await
                {
                    ui.note(&format!("SESSION_ENDED {end:?}"));
                    // To the person as well, where the note went elsewhere.
                    if !began && (ui.log.is_some() || std::io::stderr().is_terminal()) {
                        eprintln!("SESSION_ENDED {end:?}");
                    }
                }
                connection.close(0, b"");
                let _ = tokio::time::timeout(Duration::from_millis(500), connection.closed()).await;
                played
            }
            Err(error) => Err(error.into()),
        };
        match played {
            Ok(Ended::Quit) => return Ok(()),
            Ok(Ended::Again) if room.is_some() => {}
            Ok(Ended::Again) => return Ok(()),
            // WHY the first try ends the viewer and a later one does not: a relay that turns
            // the viewer down is to be seen, and a broadcast that broke off may come back.
            Err(error) if room.is_none() || (first && !began) => return Err(error),
            Err(error) => {
                ui.note(&format!("PLAY_ENDED {error:#}"));
                // The room may still name the broadcast that is gone: not at once again.
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(1)) => {}
                    asked = ui.asked() => if !matches!(asked, keys::Asked::Say(_)) { return Ok(()) },
                }
            }
        }
        first = false;
    }
}

#[allow(clippy::too_many_arguments)]
async fn play(
    args: &Args,
    connection: &Arc<WtConnection>,
    namespace: &str,
    token: Option<&[u8]>,
    room: &mut Option<room::Room>,
    playing: Option<&room::Broadcast>,
    ui: &mut Ui,
    began_playing: &mut bool,
) -> Result<Ended> {
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let stops = StreamStops::default();
    let (queue, mut events) = mpsc::channel(64);
    let _streams = read_uni_streams(Arc::clone(connection), StreamConfig::default(), stops.clone(), queue, |event| event);
    // §3.3: each peer opens one control stream that begins with SETUP. It stays open.
    let mut control = connection.open_uni().await?;
    // In ascending order of their types: the token (0x03), then the implementation (0x07).
    let mut options = Vec::new();
    if let Some(token) = token {
        options.push(KeyValuePair::bytes(setup_option::AUTHORIZATION_TOKEN, token.to_vec())?);
    }
    options.push(KeyValuePair::bytes(setup_option::MOQT_IMPLEMENTATION, b"xmoq-watch (spike)".to_vec())?);
    let setup = session.setup(options)?;
    send_message(&mut control, &setup, &Trace::default(), "control").await?;
    match events.recv().await {
        Some(UniEvent::Control(message)) => session.on_control_message(message)?,
        other => bail!("no SETUP from the relay: {other:?}"),
    };

    let namespace = TrackNamespace::new(namespace.split('/').map(|field| field.as_bytes().to_vec()).collect())?;
    let (video_name, audio_name, _catalog) = match (&args.video, &args.audio) {
        (Some(video), Some(audio)) => (video.clone(), audio.clone(), None),
        (None, None) => {
            let (tracks, kept) = catalog::read(connection, &mut session, &mut events, &stops, &namespace, token).await?;
            let video = xmoq_msf::first_video_track(&tracks, "avc").context("the catalog has no H.264 video track with LOC packaging")?;
            let audio = xmoq_msf::first_audio_track(&tracks, "opus").context("the catalog has no Opus audio track with LOC packaging")?;
            // WHY said and not acted on: Opus is decoded at 48 kHz whatever it was made from,
            // and the decoder mixes two channels into the one that is played.
            let line = format!(
                "CATALOG tracks={} video={} codec={} size={}x{} audio={} codec={} samplerate={} channel_config={}",
                tracks.len(),
                video.name,
                video.codec.as_deref().unwrap_or("-"),
                video.width.unwrap_or(0),
                video.height.unwrap_or(0),
                audio.name,
                audio.codec.as_deref().unwrap_or("-"),
                audio.samplerate.map_or("-".to_string(), |rate| rate.to_string()),
                audio.channel_config.as_deref().unwrap_or("-"),
            );
            ui.note(&line.chars().filter(|char| !char.is_control()).collect::<String>());
            (video.name.clone(), audio.name.clone(), Some(kept))
        }
        _ => bail!("--video and --audio are given together, or neither"),
    };
    let mut video = subscribe(connection, &mut session, &namespace, &video_name, token).await?;
    let mut audio = subscribe(connection, &mut session, &namespace, &audio_name, token).await?;

    let to: Box<dyn std::io::Write + std::marker::Send> = match &args.out {
        Some(path) => Box::new(std::fs::OpenOptions::new().create(true).append(true).open(path)?),
        None => Box::new(std::io::stdout()),
    };
    let screen = screen::Screen::new(to, args.medium, args.scale, Arc::clone(&ui.under));
    let (frames, for_picture) = std_mpsc::channel();
    let picture = std::thread::spawn(move || picture::run(for_picture, screen));
    let mut sound = sound::Sound::open(Arc::clone(&ui.volume))?;
    ui.show();

    let began = Instant::now();
    let mut clock = Clock { began, least: None, buffer: args.buffer };
    let (mut no_timestamp, mut streams_reset) = (0u64, 0u64);
    let mut ended = Ended::Again;
    let mut failed = None;
    let why = loop {
        tokio::select! {
            asked = ui.asked() => match asked {
                keys::Asked::Say(text) => match room.as_mut() {
                    Some(room) => {
                        ui.note(&format!("SAID {text}"));
                        room.say(&text).await?;
                    }
                    None => ui.note("SAID nothing: no room"),
                },
                _ => {
                    ended = Ended::Quit;
                    break "the viewer was ended";
                }
            },
            said = async {
                match room.as_mut() {
                    Some(room) => room.next().await,
                    None => std::future::pending().await,
                }
            } => match said {
                Ok(room::Said::Chat(line)) => ui.chat(line),
                Ok(room::Said::Broadcast(broadcast)) => {
                    if broadcast.is_some() && broadcast.as_ref() != playing {
                        break "the room has another broadcast";
                    }
                    if broadcast.is_none() {
                        ui.note("ROOM_BROADCAST none");
                    }
                }
                // The room is gone; the broadcast is watched on without it.
                Err(error) => {
                    ui.note(&format!("ROOM_LOST {error:#}"));
                    *room = None;
                }
            },
            event = video.reader.next_event() => {
                match event {
                    Ok(RequestStreamEvent::Message(message)) => {
                        session.on_request_message(video.request, message)?;
                    }
                    Ok(RequestStreamEvent::Closed(_)) => {}
                    Err(error) => failed = Some(error.into()),
                }
                break "the video track ended";
            }
            event = audio.reader.next_event() => {
                match event {
                    Ok(RequestStreamEvent::Message(message)) => {
                        session.on_request_message(audio.request, message)?;
                    }
                    Ok(RequestStreamEvent::Closed(_)) => {}
                    Err(error) => failed = Some(error.into()),
                }
                break "the audio track ended";
            }
            event = events.recv() => match event {
                Some(UniEvent::Object { header, object, .. }) => {
                    if object.status != ObjectStatus::Normal {
                        continue;
                    }
                    let Some(timestamp) = timestamp_of(&object) else {
                        no_timestamp += 1;
                        continue;
                    };
                    *began_playing = true;
                    let due = clock.due(timestamp);
                    if header.track_alias == video.track_alias {
                        let annex_b = loc::to_annex_b(&object.payload).map_err(|error| anyhow!("a video payload: {}", error.0))?;
                        let _ = frames.send(picture::Frame { group: header.group_id, object: object.object_id, annex_b, due });
                    } else if header.track_alias == audio.track_alias {
                        // WHY here and not on a thread of its own: 2.5 ms for a packet of 20 ms.
                        sound.take(&object.payload, due);
                    }
                }
                Some(UniEvent::SubgroupClosed { header, end, .. }) => {
                    session.on_data_stream_closed(header.track_alias);
                    if !matches!(end, xmoq_moqt_io::StreamEnd::Fin) {
                        streams_reset += 1;
                    }
                }
                Some(UniEvent::SubgroupOpened { .. }) => {}
                Some(UniEvent::Failed(error)) => {
                    failed = Some(error.into());
                    break "the session failed";
                }
                Some(other) => bail!("not expected of the relay: {other:?}"),
                None => {
                    failed = Some(anyhow!("the connection is gone"));
                    break "the connection is gone";
                }
            },
        }
    };
    let seconds = began.elapsed().as_secs_f64();
    drop(frames);
    let seen = picture.join().map_err(|_| anyhow!("the picture's thread panicked"))?;
    let heard = sound.counts();
    // The control stream stays open until the session is closed (§3.3).
    std::mem::forget(control);

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
    let line = format!(
        "AV_PLAYED why=\"{why}\" seconds={seconds:.1} buffer_ms={} volume={:.1} muted={} \
         video_came={} video_left_out={} video_drawn={} fps={:.1} video_off_ms_min/p50/p95/max={} decode_ms_a_frame={:.2} draw_ms_a_frame={:.2} \
         audio_packets={} audio_refused={} audio_seconds={:.2} underruns={} silence_waiting_ms={} audio_left_out_late={} audio_cut_late={} audio_dropped_full={} \
         audio_off_ms_min/p50/p95/max={} peak={:.3} rms={:.3} \
         marks_video={} marks_audio={} picture_after_sound_ms_min/p50/p95/max={} no_timestamp={no_timestamp} streams_reset={streams_reset}",
        args.buffer.as_millis(),
        ui.volume.level(),
        ui.volume.muted(),
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
    eprintln!("{line}");
    if ui.log.is_some() {
        ui.note(&line);
    }
    match failed {
        Some(error) => Err(error),
        None => Ok(ended),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    // Told to end by a signal: the loop ends, and what is below puts the terminal back.
    let (stopping, stop) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut terminate), Ok(mut hangup)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup())) else {
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
        let _ = stopping.send(true);
    });
    // The terminal is put back when `_raw` is dropped, on every way out of this function.
    let (_raw, pressed) = match keys::start() {
        Some((raw, pressed)) => (Some(raw), Some(pressed)),
        None => (None, None),
    };
    // For the check that a panic puts the terminal back (try-keys.sh).
    if std::env::var_os("XMOQ_WATCH_PANIC").is_some() {
        tokio::time::sleep(Duration::from_secs(1)).await;
        panic!("asked to panic");
    }
    let mut ui = Ui {
        under: Arc::default(),
        volume: Arc::new(sound::Volume::new(args.volume)),
        keys: keys::Keys::default(),
        pressed,
        log: match &args.log {
            Some(path) => Some(std::fs::File::create(path)?),
            None => None,
        },
        stop,
        deadline: args.seconds.map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds)),
    };
    run(&args, &mut ui).await
}
