//! A viewer for the terminal: subscribes to a video track and an audio
//! track on one session, and plays both by the Timestamps of LOC: the
//! picture with the Kitty graphics protocol, the sound on the machine's
//! output. With a room, the room says where the broadcast is and gives a
//! token, its chat is written under the picture, and the viewer follows a
//! broadcast that ends and begins again. `xmoq_watch::args::USAGE` has the
//! flags and the keys.
//!
//! What it decides is in the library of this crate, without input or
//! output; the files here do the input and output, each one kind of it.

mod audio_output;
mod picture;
mod room_socket;
mod session;
mod sound;
mod terminal;
mod video_decoder;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::ObjectStatus;
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::setup_option;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{Role, Session, Transport};
use xmoq_moqt_io::wt::{self, WtConnection};
use xmoq_moqt_io::{
    Connection, RequestStreamEvent, StreamConfig, StreamEnd, StreamStops, Trace, UniEvent,
    read_uni_streams, send_message,
};
use xmoq_watch::args::{Args, Fit, Source};
use xmoq_watch::clock::Clock;
use xmoq_watch::keys::{Asked, Keys};
use xmoq_watch::room::{self, Broadcast};
use xmoq_watch::{marks, tracks};

use crate::picture::{Frame, LINES, Screen, Seen, Under};
use crate::room_socket::{Heard, RoomSocket};
use crate::session::{Requests, Subscription, timestamp_of};
use crate::sound::{Sound, Volume, micros_since};
use crate::video_decoder::VideoDecoder;

/// What the viewer shares between its parts while it runs.
struct Ui {
    under: Arc<Mutex<Under>>,
    volume: Arc<Volume>,
    keys: Keys,
    pressed: Option<mpsc::Receiver<u8>>,
    log: Option<std::fs::File>,
    /// Becomes true when the process is told to end.
    stop: tokio::sync::watch::Receiver<bool>,
    deadline: Option<tokio::time::Instant>,
}

impl Ui {
    /// A line about the room or the keys: to `--log`, or to stderr where
    /// that is no terminal. WHY not to a terminal: the picture is there.
    fn note(&mut self, line: &str) {
        if let Some(log) = &mut self.log {
            let _ = writeln!(log, "{line}");
        } else if !std::io::stderr().is_terminal() {
            eprintln!("{line}");
        }
    }

    fn chat(&mut self, line: String) {
        self.note(&format!("CHAT {line}"));
        let mut under = self.under.lock().unwrap_or_else(PoisonError::into_inner);
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
                if self.volume.muted() {
                    " (muted, [m])"
                } else {
                    "  [m] mute"
                }
            ),
        };
        let mut under = self.under.lock().unwrap_or_else(PoisonError::into_inner);
        under.status = room::clean(&status, room::LINE_LIMIT);
        under.version += 1;
    }

    /// The next thing that is not the broadcast's: a key that asks for
    /// something beyond the volume, the end of the time, or the order to
    /// end.
    async fn asked(&mut self) -> Asked {
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
                () = deadline => return Asked::Quit,
                _ = self.stop.changed() => return Asked::Quit,
                byte = pressed => {
                    // Stdin is over: no more keys.
                    let Some(byte) = byte else {
                        self.pressed = None;
                        continue;
                    };
                    let asked = self.keys.press(byte);
                    match &asked {
                        Some(Asked::Louder) => self.volume.change(0.1),
                        Some(Asked::Quieter) => self.volume.change(-0.1),
                        Some(Asked::Mute) => self.volume.mute(),
                        _ => {}
                    }
                    self.show();
                    match asked {
                        Some(Asked::Louder | Asked::Quieter | Asked::Mute) => {
                            let line = format!(
                                "VOLUME level={:.1} muted={}",
                                self.volume.level(),
                                self.volume.muted()
                            );
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

/// Where one broadcast is played from.
struct Place {
    url: String,
    cert_sha256: [u8; 32],
    namespace: String,
    token: Option<Vec<u8>>,
    /// The broadcast of the room this is, where a room said it.
    broadcast: Option<Broadcast>,
}

/// Waits until the room has a broadcast. `None` when the viewer is to end
/// first.
async fn broadcast_of(room: &mut RoomSocket, ui: &mut Ui) -> Result<Option<Broadcast>> {
    loop {
        if let Some(broadcast) = room.current() {
            return Ok(Some(broadcast));
        }
        tokio::select! {
            heard = room.next() => match heard? {
                Heard::Chat(line) => ui.chat(line),
                Heard::Broadcast(_) => {}
            },
            asked = ui.asked() => match asked {
                Asked::Say(text) => room.say(&text).await?,
                _ => return Ok(None),
            },
        }
    }
}

/// Where the broadcast is now: of the flags, or as the room says, with a
/// token of the room. `None` when the viewer is to end before a room has
/// one.
async fn place(args: &Args, room: &mut Option<RoomSocket>, ui: &mut Ui) -> Result<Option<Place>> {
    let (room, ask_token) = match (&args.source, room) {
        (
            Source::Relay {
                url,
                cert_sha256,
                namespace,
            },
            _,
        ) => {
            return Ok(Some(Place {
                url: url.clone(),
                cert_sha256: *cert_sha256,
                namespace: namespace.clone(),
                token: None,
                broadcast: None,
            }));
        }
        (Source::Room { ask_token, .. }, Some(room)) => (room, *ask_token),
        (Source::Room { .. }, None) => bail!("the room is gone"),
    };
    if room.current().is_none() {
        ui.note("ROOM_BROADCAST none: waiting");
    }
    let Some(broadcast) = broadcast_of(room, ui).await? else {
        return Ok(None);
    };
    ui.note(&format!(
        "ROOM_BROADCAST relay={} namespace={}",
        room::clean(&broadcast.relay_url, room::LINE_LIMIT),
        room::clean(&broadcast.namespace.join("/"), room::LINE_LIMIT)
    ));
    let mut token = None;
    if ask_token {
        token = room
            .token(&broadcast.namespace)
            .await?
            .map(|text| room::authorization_token(&text));
        ui.note(&format!(
            "ROOM_TOKEN {}",
            if token.is_some() {
                "given"
            } else {
                "none: the room has no key"
            }
        ));
    }
    Ok(Some(Place {
        url: broadcast.relay_url.clone(),
        cert_sha256: room::certificate_hash(&broadcast.cert_sha256_hex)?,
        namespace: broadcast.namespace.join("/"),
        token,
        broadcast: Some(broadcast),
    }))
}

async fn run(args: &Args, ui: &mut Ui) -> Result<()> {
    // With a room: where the broadcast is, and a token, are the room's to say.
    let mut room = match &args.source {
        Source::Room {
            address, name, say, ..
        } => {
            let mut room = RoomSocket::join(address, name).await?;
            if let Some(text) = say {
                room.say(text).await?;
            }
            Some(room)
        }
        Source::Relay { .. } => None,
    };
    let follows = room.is_some();
    let mut first = true;
    loop {
        let Some(place) = place(args, &mut room, ui).await? else {
            return Ok(());
        };
        let mut began = false;
        let streams = StreamConfig::default();
        let played = match wt::connect(&place.url, place.cert_sha256, &streams).await {
            Ok(connection) => {
                let connection = Arc::new(connection);
                let played = play(args, &connection, &place, &mut room, ui, &mut began).await;
                if played.is_err()
                    && let Ok(end) =
                        tokio::time::timeout(Duration::from_millis(500), connection.closed()).await
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
            // Without a room nothing says what comes next.
            Ok(Ended::Again) if follows => {}
            Ok(Ended::Quit | Ended::Again) => return Ok(()),
            // WHY the first try ends the viewer and a later one does not: a
            // relay that turns the viewer down is to be seen, and a
            // broadcast that broke off may come back.
            Err(error) if !follows || (first && !began) => return Err(error),
            Err(error) => {
                ui.note(&format!("PLAY_ENDED {error:#}"));
                // The room may still name the broadcast that is gone: not
                // at once again.
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(1)) => {}
                    asked = ui.asked() => if !matches!(asked, Asked::Say(_)) { return Ok(()) },
                }
            }
        }
        first = false;
    }
}

/// The tracks to play: by name, or the first of each kind the catalog has.
async fn subscriptions(
    args: &Args,
    requests: &mut Requests<'_>,
    events: &mut mpsc::Receiver<UniEvent>,
    stops: &StreamStops,
    ui: &mut Ui,
) -> Result<(Subscription, Subscription, Option<session::CatalogKept>)> {
    let (video, audio, kept) = if let Some((video, audio)) = &args.tracks {
        (video.clone(), audio.clone(), None)
    } else {
        let (catalog, kept) = requests.catalog(events, stops).await?;
        let chosen = tracks::choose(&catalog)?;
        let (video, audio) = (chosen.video, chosen.audio);
        // WHY said and not acted on: Opus is decoded at 48 kHz whatever it
        // was made from, and the decoder mixes two channels into the one
        // that is played.
        let line = format!(
            "CATALOG tracks={} video={} codec={} size={}x{} audio={} codec={} samplerate={} channel_config={}",
            catalog.len(),
            video.name,
            video.codec.as_deref().unwrap_or("-"),
            video.width.unwrap_or(0),
            video.height.unwrap_or(0),
            audio.name,
            audio.codec.as_deref().unwrap_or("-"),
            audio
                .samplerate
                .map_or("-".to_string(), |rate| rate.to_string()),
            audio.channel_config.as_deref().unwrap_or("-"),
        );
        ui.note(&room::clean(&line, 4 * room::LINE_LIMIT));
        (video.name.clone(), audio.name.clone(), Some(kept))
    };
    let video = requests.subscribe(&video).await?;
    let audio = requests.subscribe(&audio).await?;
    Ok((video, audio, kept))
}

/// The screen the picture is written to.
fn screen(args: &Args, ui: &Ui) -> Result<Screen> {
    let to: Box<dyn Write + Send> = match &args.out {
        Some(path) => Box::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
        ),
        None => Box::new(std::io::stdout()),
    };
    // WHY to the terminal's window by default: a frame larger than that
    // would be scaled down by the terminal, and written in full for nothing.
    let window = if args.out.is_none() {
        terminal::window(LINES)
    } else {
        None
    };
    let cell = window.map_or((8, 16), |(_, cell)| cell);
    let fit = match args.fit {
        Fit::Terminal => window.map(|(most, _)| most),
        Fit::AsItIs => None,
        Fit::Most(width, height) => Some((width, height)),
    };
    Ok(Screen {
        to,
        medium: args.medium,
        scale: args.scale,
        fit,
        cell,
        under: Arc::clone(&ui.under),
    })
}

/// What one broadcast was, for the line at its end.
struct Played {
    why: &'static str,
    lasted: Duration,
    seen: Seen,
    heard: sound::Heard,
    sound_marks_us: Vec<u64>,
    audio_over_least_us: Vec<i64>,
    video_over_least_us: Vec<i64>,
    no_timestamp: u64,
    streams_reset: u64,
    /// What the sound output is ahead by, which is added to the buffer.
    output_lead_us: u64,
}

/// Microseconds as milliseconds with one decimal.
fn ms(us: i64) -> String {
    format!(
        "{}{}.{}",
        if us < 0 { "-" } else { "" },
        us.unsigned_abs() / 1000,
        us.unsigned_abs() % 1000 / 100
    )
}

fn spread_ms(values: &[i64]) -> String {
    marks::spread(values).map(ms).join("/")
}

impl Played {
    fn line(&self, args: &Args, ui: &Ui) -> String {
        let seen = &self.seen;
        let counts = &self.heard.counts;
        let a_frame = |all: Duration, frames: u32| (all / frames.max(1)).as_secs_f64() * 1000.0;
        let fps = if seen.shown_for.is_zero() {
            0.0
        } else {
            f64::from(seen.drawn.saturating_sub(1)) / seen.shown_for.as_secs_f64()
        };
        let apart = marks::picture_after_sound_us(&seen.marks_us, &self.sound_marks_us);
        let samples_ms = |samples: u64| samples * 1000 / u64::from(xmoq_watch::playout::RATE);
        format!(
            "AV_PLAYED why=\"{}\" seconds={:.1} buffer_ms={} output_lead_ms={} volume={:.1} muted={} \
             video_size={}x{} made_smaller_by={} written_kb_a_frame={} video_came={} video_left_out={} video_drawn={} fps={fps:.1} \
             video_after_due_ms_min/p50/p95/max={} decode_ms_a_frame={:.2} draw_ms_a_frame={:.2} \
             audio_packets={} audio_refused={} audio_played_ms={} underruns={} silence_waiting_ms={} audio_left_out_late={} audio_cut_late={} audio_dropped_full={} \
             peak={:.3} rms={:.3} audio_came_over_least_ms_min/p50/p95/max={} video_came_over_least_ms_min/p50/p95/max={} \
             marks_video={} marks_audio={} picture_after_sound_ms_min/p50/p95/max={} no_timestamp={} streams_reset={}",
            self.why,
            self.lasted.as_secs_f64(),
            args.buffer_ms,
            ms(i64::try_from(self.output_lead_us).unwrap_or(i64::MAX)),
            ui.volume.level(),
            ui.volume.muted(),
            seen.size.0,
            seen.size.1,
            seen.by,
            seen.bytes / 1024 / u64::from(seen.drawn.max(1)),
            seen.came,
            seen.left_out,
            seen.drawn,
            spread_ms(&seen.after_due_us),
            a_frame(seen.decoding, seen.decoded),
            a_frame(seen.drawing, seen.drawn),
            counts.packets,
            self.heard.refused,
            samples_ms(counts.played),
            counts.underruns,
            samples_ms(counts.waited),
            counts.left_out_late,
            counts.cut_late,
            counts.dropped_full,
            self.heard.peak,
            self.heard.rms,
            spread_ms(&self.audio_over_least_us),
            spread_ms(&self.video_over_least_us),
            seen.marks_us.len(),
            self.sound_marks_us.len(),
            spread_ms(&apart),
            self.no_timestamp,
            self.streams_reset,
        )
    }
}

/// Plays one broadcast until it ends, moves, or the viewer is ended.
/// `began_playing` is set when its first Object came.
#[expect(
    clippy::too_many_lines,
    reason = "one select over everything that can happen while a broadcast plays: cut in two, \
              each half would need the other's state"
)]
async fn play(
    args: &Args,
    connection: &Arc<WtConnection>,
    place: &Place,
    room: &mut Option<RoomSocket>,
    ui: &mut Ui,
    began_playing: &mut bool,
) -> Result<Ended> {
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let stops = StreamStops::default();
    let (queue, mut events) = mpsc::channel(64);
    let _streams = read_uni_streams(
        Arc::clone(connection),
        StreamConfig::default(),
        stops.clone(),
        queue,
        |event| event,
    );
    // §3.3: each peer opens one control stream that begins with SETUP. It
    // stays open: closing it is a PROTOCOL_VIOLATION for the peer.
    let mut control = connection.open_uni().await?;
    // In ascending order of their types: the token (0x03), then the
    // implementation (0x07).
    let mut options = Vec::new();
    if let Some(token) = &place.token {
        options.push(KeyValuePair::bytes(
            setup_option::AUTHORIZATION_TOKEN,
            token.clone(),
        )?);
    }
    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    options.push(KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        implementation.as_bytes().to_vec(),
    )?);
    let setup = session.setup(options)?;
    send_message(&mut control, &setup, &Trace::default(), "control").await?;
    match events.recv().await {
        Some(UniEvent::Control(message)) => session.on_control_message(message)?,
        other => bail!("no SETUP from the relay: {other:?}"),
    };

    let namespace = TrackNamespace::new(
        place
            .namespace
            .split('/')
            .map(|field| field.as_bytes().to_vec())
            .collect(),
    )?;
    let mut requests = Requests {
        connection,
        session: &mut session,
        namespace: &namespace,
        token: place.token.as_deref(),
    };
    let (mut video, mut audio, _catalog) =
        subscriptions(args, &mut requests, &mut events, &stops, ui).await?;

    let began = Instant::now();
    let (frames, for_picture) = std_mpsc::channel();
    let (screen, decoder) = (screen(args, ui)?, VideoDecoder::new()?);
    let measure = args.measure_demo_marks;
    let picture =
        std::thread::spawn(move || picture::run(for_picture, screen, decoder, began, measure));
    let mut sound = Sound::open(Arc::clone(&ui.volume), began)?;
    ui.show();

    // WHY the lead of the sound output is added to the buffer, for the
    // picture as for the sound: the output asks for a sample that long
    // before it is due, so without it that much of `--buffer-ms` is gone
    // before a packet may be late at all (24 of 50 ms on the machine this
    // was written on). Added to both, the two stay together.
    let lead_us = sound.lead_us().await;
    let mut clock = Clock::new(args.buffer_ms.saturating_mul(1000).saturating_add(lead_us));
    let mut played = Played {
        why: "",
        lasted: Duration::ZERO,
        seen: Seen::default(),
        heard: sound.heard(),
        sound_marks_us: Vec::new(),
        audio_over_least_us: Vec::new(),
        video_over_least_us: Vec::new(),
        no_timestamp: 0,
        streams_reset: 0,
        output_lead_us: lead_us,
    };
    let mut ended = Ended::Again;
    let mut failed = None;
    played.why = loop {
        tokio::select! {
            asked = ui.asked() => {
                let Asked::Say(text) = asked else {
                    ended = Ended::Quit;
                    break "the viewer was ended";
                };
                if let Some(room) = room.as_mut() {
                    ui.note(&format!("SAID {}", room::clean(&text, room::LINE_LIMIT)));
                    room.say(&text).await?;
                } else {
                    ui.note("SAID nothing: no room");
                }
            }
            heard = async {
                match room.as_mut() {
                    Some(room) => room.next().await,
                    None => std::future::pending().await,
                }
            } => match heard {
                Ok(Heard::Chat(line)) => ui.chat(line),
                Ok(Heard::Broadcast(broadcast)) => {
                    if broadcast.is_some() && broadcast != place.broadcast {
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
                        played.no_timestamp += 1;
                        continue;
                    };
                    *began_playing = true;
                    let due = clock.due(micros_since(began, Instant::now()), timestamp);
                    let over_least = i64::try_from(due.over_least_us).unwrap_or(i64::MAX);
                    if header.track_alias == video.track_alias {
                        played.video_over_least_us.push(over_least);
                        let annex_b = loc::to_annex_b(&object.payload)
                            .map_err(|error| anyhow!("a video payload: {}", error.0))?;
                        let _ = frames.send(Frame {
                            group: header.group_id,
                            object: object.object_id,
                            annex_b,
                            due: began + Duration::from_micros(due.at_us),
                        });
                    } else if header.track_alias == audio.track_alias {
                        played.audio_over_least_us.push(over_least);
                        if sound.take(&object.payload, due.at_us) && measure {
                            played.sound_marks_us.push(due.at_us);
                        }
                    }
                }
                Some(UniEvent::SubgroupClosed { header, end, .. }) => {
                    session.on_data_stream_closed(header.track_alias);
                    if !matches!(end, StreamEnd::Fin) {
                        played.streams_reset += 1;
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
    sound.end();
    played.lasted = sound.began().elapsed();
    drop(frames);
    played.seen = picture
        .join()
        .map_err(|_| anyhow!("the picture's thread panicked"))?;
    played.heard = sound.heard();
    // WHY the control stream is not let go of here: a stream that is
    // dropped is finished, which the relay takes for a PROTOCOL_VIOLATION
    // (§3.3) before the close of the session reaches it. It goes with the
    // connection, which the caller closes.
    std::mem::forget(control);

    let line = played.line(args, ui);
    eprintln!("{line}");
    if ui.log.is_some() {
        ui.note(&line);
    }
    match failed {
        Some(error) => Err(error),
        None => Ok(ended),
    }
}

async fn watch(args: Args) -> Result<()> {
    // Told to end by a signal: the loop ends, and what is below puts the
    // terminal back.
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
    // The terminal is put back when `_raw` is dropped, on every way out of
    // this function.
    let (_raw, pressed) = match terminal::keys() {
        Some((raw, pressed)) => (Some(raw), Some(pressed)),
        None => (None, None),
    };
    let mut ui = Ui {
        under: Arc::default(),
        volume: Arc::new(Volume::new(args.volume)),
        keys: Keys::default(),
        pressed,
        log: match &args.log {
            Some(path) => {
                Some(std::fs::File::create(path).with_context(|| format!("--log {path}"))?)
            }
            None => None,
        },
        stop,
        deadline: args
            .seconds
            .map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds)),
    };
    run(&args, &mut ui).await
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match xmoq_watch::args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n{}", xmoq_watch::args::USAGE);
            return ExitCode::from(2);
        }
    };
    match watch(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
