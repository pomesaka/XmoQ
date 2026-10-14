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

use std::future::Future;
use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
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
use xmoq_watch::again::AskAgain;
use xmoq_watch::args::{Args, Fit, Source};
use xmoq_watch::clock::{AUTO_MOST_US, Clock};
use xmoq_watch::keys::{Asked, Keys};
use xmoq_watch::marks::Recent;
use xmoq_watch::room::{self, Broadcast};
use xmoq_watch::{marks, tracks};

use crate::picture::{
    Frame, LINES, MOST_COUNTED, MOST_FRAMES_WAITING, Picture, Screen, Seen, Under,
};
use crate::room_socket::{Heard, RoomSocket};
use crate::session::{NotSubscribed, Requests, Sent, Subscription, timestamp_of};
use crate::sound::{Sound, Volume, micros_since};
use crate::video_decoder::VideoDecoder;

/// How often the automatic buffer is moved: twice a second, as the
/// watching page's (`packages/media/src/core/playout-clock.ts`).
const BUFFER_TICK: Duration = Duration::from_millis(500);

/// How long the relay gets for its SETUP, a room to take the viewer in, and
/// either to be connected to. **Provisional.** The answer to a request has
/// its own time (`xmoq_moqt::params::answer_wait_ms`, in `session.rs`), and
/// the catalog is waited 5 seconds for.
const ANSWER_WAIT: Duration = Duration::from_secs(10);
/// More than the waits for the relay at the beginning of a broadcast come
/// to, each of which has its own time: the SETUP (10 s), the catalog (the
/// answer to its SUBSCRIBE, 10 s, and the catalog itself, 5 s), and the
/// answers to the SUBSCRIBEs of the video and of the sound (10 s each).
const BEGINNING_AT_MOST: Duration = Duration::from_secs(60);

/// What has to stay until the session is closed, on every way out of
/// [`play`], so its caller keeps it.
///
/// WHY: a stream that is let go of is finished or stopped, and the close
/// of the session takes a moment to reach the relay. A relay that sees the
/// viewer's control stream end, or its own stopped, closes the session
/// first, with `PROTOCOL_VIOLATION` (§3.3).
#[derive(Default)]
struct UntilClosed {
    control: Option<<WtConnection as Connection>::Send>,
    readers: Option<xmoq_moqt_io::IncomingStreams>,
}

/// The relay sent no SETUP in so many milliseconds.
#[derive(Debug)]
struct NoSetupInTime(u64);

impl std::fmt::Display for NoSetupInTime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the relay sent no SETUP within {} ms", self.0)
    }
}

impl std::error::Error for NoSetupInTime {}
/// How long a room gets for a token, and to take a line of the chat.
/// **Provisional.**
const ROOM_WAIT: Duration = Duration::from_secs(5);
/// How long the thread of the picture gets to end. **Provisional**: it
/// hears of the end within 20 ms unless the terminal does not take what it
/// writes.
const PICTURE_WAIT: Duration = Duration::from_secs(2);
/// The least time from one try at a broadcast to the next, however often a
/// room names another. **Provisional.**
const BETWEEN_TRIES: Duration = Duration::from_secs(1);
/// The most lines of the chat that are kept for the lines under the picture.
const CHAT_KEPT: usize = 16;
/// The most characters of a line this program writes: an error with what
/// a peer said in it, the tracks of a catalog, and `AV_PLAYED`, which is
/// the longest at about 1,100.
const LINE_ABOUT_A_PEER: usize = 2000;

/// Writes a line to stderr with nothing in it a terminal would act on.
///
/// WHY every line goes through here or through [`Ui::note`]: a line can
/// carry what a peer chose (a track's name in an error, a reason a relay
/// gave), and stderr is a terminal more often than not.
fn say(line: &str) {
    eprintln!("{}", room::clean(line, LINE_ABOUT_A_PEER));
}

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
            let _ = writeln!(log, "{}", room::clean(line, LINE_ABOUT_A_PEER));
        } else if !std::io::stderr().is_terminal() {
            say(line);
        }
    }

    /// Waits for what a peer is to do, for no longer than `limit`, and not
    /// at all once the viewer is told to end: `None` then.
    ///
    /// WHY everything a peer is waited for goes through here: a peer that
    /// does not answer must not be what keeps `q`, Ctrl-C, a signal or
    /// `--seconds` from ending the viewer.
    async fn unless_ended<T>(
        &mut self,
        what: &str,
        limit: Duration,
        doing: impl Future<Output = Result<T>>,
    ) -> Result<Option<T>> {
        tokio::pin!(doing);
        let late = tokio::time::sleep(limit);
        tokio::pin!(late);
        loop {
            tokio::select! {
                done = &mut doing => return done.map(Some),
                () = &mut late => bail!("{what} took more than {} s", limit.as_secs()),
                // A line for the chat that is typed meanwhile is let go.
                asked = self.asked() => if !matches!(asked, Asked::Say(_)) {
                    return Ok(None);
                },
            }
        }
    }

    fn chat(&mut self, line: String) {
        self.note(&format!("CHAT {line}"));
        let mut under = self.under.lock().unwrap_or_else(PoisonError::into_inner);
        under.chat.push(line);
        // The last few are shown; a chat that goes on is not kept.
        let beyond = under.chat.len().saturating_sub(CHAT_KEPT);
        under.chat.drain(..beyond);
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
    /// The fields of the Track Namespace.
    namespace: Vec<String>,
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
                // The flag has the fields between slashes.
                namespace: namespace.split('/').map(str::to_string).collect(),
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
        let Some(given) = ui
            .unless_ended(
                "the room's token",
                ROOM_WAIT,
                room.token(&broadcast.namespace),
            )
            .await?
        else {
            return Ok(None);
        };
        token = given.map(|text| room::authorization_token(&text));
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
        namespace: broadcast.namespace.clone(),
        token,
        broadcast: Some(broadcast),
    }))
}

async fn run(args: &Args, ui: &mut Ui) -> Result<()> {
    // With a room: where the broadcast is, and a token, are the room's to say.
    let mut room = match &args.source {
        Source::Room {
            address,
            name,
            say: line,
            ..
        } => {
            let joining = RoomSocket::join(address, name);
            let Some(mut room) = ui.unless_ended("the room", ANSWER_WAIT, joining).await? else {
                return Ok(());
            };
            if let Some(text) = line {
                tokio::time::timeout(ROOM_WAIT, room.say(text))
                    .await
                    .map_err(|_| anyhow!("the room did not take a line for the chat"))??;
            }
            Some(room)
        }
        Source::Relay { .. } => None,
    };
    let follows = room.is_some();
    let mut first = true;
    let mut tried: Option<Instant> = None;
    loop {
        let Some(place) = place(args, &mut room, ui).await? else {
            return Ok(());
        };
        // WHY not at once again: a room that names one broadcast after the
        // other, or one that is gone, would have the viewer connect as fast
        // as it can.
        if let Some(wait) = tried.and_then(|tried| BETWEEN_TRIES.checked_sub(tried.elapsed())) {
            let waiting = async {
                tokio::time::sleep(wait).await;
                Ok(())
            };
            if ui
                .unless_ended("the wait", BETWEEN_TRIES * 2, waiting)
                .await?
                .is_none()
            {
                return Ok(());
            }
        }
        tried = Some(Instant::now());
        let mut began = false;
        let streams = StreamConfig::default();
        let connecting = async { Ok(wt::connect(&place.url, place.cert_sha256, &streams).await?) };
        let played = match ui.unless_ended("the relay", ANSWER_WAIT, connecting).await {
            Ok(None) => return Ok(()),
            Ok(Some(connection)) => {
                let connection = Arc::new(connection);
                let mut kept = UntilClosed::default();
                let played = play(
                    args,
                    &connection,
                    &place,
                    &mut room,
                    ui,
                    &mut began,
                    &mut kept,
                )
                .await;
                if played.is_err()
                    && let Ok(end) =
                        tokio::time::timeout(Duration::from_millis(500), connection.closed()).await
                {
                    ui.note(&format!("SESSION_ENDED {end:?}"));
                    // To the person as well, where the note went elsewhere.
                    if !began && (ui.log.is_some() || std::io::stderr().is_terminal()) {
                        say(&format!("SESSION_ENDED {end:?}"));
                    }
                }
                // §3.5: "CONTROL_MESSAGE_TIMEOUT (0x11): The session was
                // closed because the peer took too long to respond to a
                // control message." For the SETUP, as `@xmoq/moqt` has it; a
                // request that is not answered is given up alone, and the
                // viewer then leaves with NO_ERROR.
                let no_setup = played
                    .as_ref()
                    .is_err_and(anyhow::Error::is::<NoSetupInTime>);
                connection.close(if no_setup { 0x11 } else { 0 }, b"");
                let _ = tokio::time::timeout(Duration::from_millis(500), connection.closed()).await;
                played
            }
            Err(error) => Err(error),
        };
        match played {
            // Without a room nothing says what comes next.
            Ok(Ended::Again) if follows => {}
            Ok(Ended::Quit | Ended::Again) => return Ok(()),
            // WHY the first try ends the viewer and a later one does not: a
            // relay that turns the viewer down is to be seen, and a
            // broadcast that broke off may come back.
            Err(error) if !follows || (first && !began) => return Err(error),
            Err(error) => ui.note(&format!("PLAY_ENDED {error:#}")),
        }
        first = false;
    }
}

/// The subscriptions of a broadcast.
struct Subscribed {
    video: Subscription,
    audio: Subscription,
    /// The name of the sound's track, to ask for it again.
    audio_name: String,
    /// What keeps the catalog's requests open.
    _catalog: Option<session::CatalogKept>,
    /// What the catalog had, as a line for the log; `None` where the tracks
    /// were named.
    found: Option<String>,
}

/// The tracks to play: by name, or the first of each kind the catalog has.
async fn subscriptions(
    args: &Args,
    requests: &mut Requests<'_>,
    events: &mut mpsc::Receiver<UniEvent>,
    stops: &StreamStops,
) -> Result<Subscribed> {
    let (video, audio, kept, found) = if let Some((video, audio)) = &args.tracks {
        (video.clone(), audio.clone(), None, None)
    } else {
        let (catalog, kept) = requests.catalog(events, stops).await?;
        let chosen = tracks::choose(&catalog, &args.video_version)?;
        let (video, audio) = (chosen.video, chosen.audio);
        // WHY said and not acted on: Opus is decoded at 48 kHz whatever it
        // was made from, and the decoder mixes two channels into the one
        // that is played.
        let line = format!(
            "CATALOG tracks={} video={} codec={} size={}x{} audio={} codec={} samplerate={} channel_config={}",
            catalog.len(),
            room::clean(&video.name, 60),
            room::clean(video.codec.as_deref().unwrap_or("-"), 30),
            video.width.unwrap_or(0),
            video.height.unwrap_or(0),
            room::clean(&audio.name, 60),
            room::clean(audio.codec.as_deref().unwrap_or("-"), 30),
            audio
                .samplerate
                .map_or("-".to_string(), |rate| rate.to_string()),
            room::clean(audio.channel_config.as_deref().unwrap_or("-"), 30),
        );
        (
            video.name.clone(),
            audio.name.clone(),
            Some(kept),
            Some(line),
        )
    };
    Ok(Subscribed {
        // WHY the video alone: a Group of it is a second of frames that
        // depend on each other, and one that is late is not worth waiting
        // for; the sound's are 200 ms and small, and losing one is heard.
        video: requests.subscribe(&video, args.video_timeout_ms).await?,
        audio: requests.subscribe(&audio, None).await?,
        audio_name: audio,
        _catalog: kept,
        found,
    })
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
    audio_over_least_us: Recent,
    video_over_least_us: Recent,
    /// The least, the most and the last the buffer was, without the lead
    /// of the output.
    buffer_us: (u64, u64, u64),
    no_timestamp: u64,
    /// Objects whose Timestamp was too far from the clock to be of it.
    not_of_the_clock: u64,
    /// Frames left out because too many waited to be drawn.
    frames_dropped_full: u64,
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
            "AV_PLAYED why=\"{}\" seconds={:.1} buffer_ms={} buffer_auto={} buffer_ms_least/most={}/{} output_lead_ms={} volume={:.1} muted={} \
             video_size={}x{} made_smaller_by={} written_kb_a_frame={} video_came={} video_left_out={} video_drawn={} fps={fps:.1} \
             video_after_due_ms_min/p50/p95/max={} decode_ms_a_frame={:.2} draw_ms_a_frame={:.2} \
             audio_packets={} audio_refused={} audio_played_ms={} underruns={} silence_waiting_ms={} audio_left_out_late={} audio_cut_late={} audio_dropped_full={} audio_stretched={} \
             peak={:.3} rms={:.3} audio_came_over_least_ms_min/p50/p95/max={} video_came_over_least_ms_min/p50/p95/max={} \
             marks_video={} marks_audio={} picture_after_sound_ms_min/p50/p95/max={} no_timestamp={} not_of_the_clock={} \
             frames_dropped_full={} pictures_too_large={} streams_reset={}",
            self.why,
            self.lasted.as_secs_f64(),
            self.buffer_us.2 / 1000,
            args.buffer_auto,
            self.buffer_us.0 / 1000,
            self.buffer_us.1 / 1000,
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
            counts.stretched,
            self.heard.peak,
            self.heard.rms,
            spread_ms(&self.audio_over_least_us.to_vec()),
            spread_ms(&self.video_over_least_us.to_vec()),
            seen.marks_us.len(),
            self.sound_marks_us.len(),
            spread_ms(&apart),
            self.no_timestamp,
            self.not_of_the_clock,
            self.frames_dropped_full,
            seen.too_large,
            self.streams_reset,
        )
    }
}

/// A SUBSCRIBE for the sound that was sent again, on its way to its answer.
type Answer = std::pin::Pin<Box<dyn Future<Output = (session::Asked, Result<Sent>)> + Send>>;

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
    kept: &mut UntilClosed,
) -> Result<Ended> {
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let stops = StreamStops::default();
    let (queue, mut events) = mpsc::channel(64);
    kept.readers = Some(read_uni_streams(
        Arc::clone(connection),
        StreamConfig::default(),
        stops.clone(),
        queue,
        |event| event,
    ));
    let control_kept = &mut kept.control;
    let namespace = TrackNamespace::new(
        place
            .namespace
            .iter()
            .map(|field| field.as_bytes().to_vec())
            .collect(),
    )?;
    // Each wait for the relay has its own time: the SETUP 10 seconds, the
    // answer to each SUBSCRIBE what `answer_wait_ms` says, the catalog 5.
    // WHY all of it under `unless_ended`: the viewer has to end when it is
    // told to at any point of it.
    let beginning = async {
        // §3.3: each peer opens one control stream that begins with SETUP.
        // It stays open: closing it is a PROTOCOL_VIOLATION for the peer.
        let control = control_kept.insert(connection.open_uni().await?);
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
        send_message(control, &setup, &Trace::default(), "control").await?;
        // 10 seconds, as `@xmoq/moqt` gives the peer's SETUP.
        let setup_wait = Duration::from_millis(args.setup_wait_ms);
        match tokio::time::timeout(setup_wait, events.recv()).await {
            Ok(Some(UniEvent::Control(message))) => session.on_control_message(message)?,
            Ok(other) => bail!("no SETUP from the relay: {other:?}"),
            Err(_) => return Err(NoSetupInTime(args.setup_wait_ms).into()),
        };
        let mut requests = Requests {
            connection,
            session: &mut session,
            namespace: &namespace,
            token: place.token.as_deref(),
            answer_wait_ms: args.answer_wait_ms,
            open_wait_ms: args.open_wait_ms,
            rendezvous_ms: args.rendezvous_ms,
        };
        let subscribed = subscriptions(args, &mut requests, &mut events, &stops).await?;
        Ok(subscribed)
    };
    let Some(subscribed) = ui
        // Each of the three SUBSCRIBEs may be held that much longer.
        .unless_ended(
            "the relay's answers",
            BEGINNING_AT_MOST + Duration::from_millis(args.rendezvous_ms.unwrap_or(0)) * 3,
            beginning,
        )
        .await?
    else {
        return Ok(Ended::Quit);
    };
    let Subscribed {
        mut video,
        audio,
        audio_name,
        _catalog,
        found,
    } = subscribed;
    // WHY the sound is asked for again on this session, and the picture
    // goes on: a relay may end the sound's subscription alone (it ends the
    // one that falls behind), and a new session would stop the picture for
    // the sake of the sound. The watching page does the same
    // (`keepTrackOnSession` of `@xmoq/media`), by the same rule
    // (`xmoq_watch::again`).
    let mut audio = Some(audio);
    let mut again = AskAgain::default();
    let mut ask_at: Option<tokio::time::Instant> = None;
    let mut asking: Option<Answer> = None;
    let mut cut_at = args
        .cut_sound_s
        .map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds));
    if let Some(line) = found {
        ui.note(&line);
    }

    let began = Instant::now();
    // WHY a queue with an end: frames come at the publisher's pace, and
    // are drawn at the terminal's.
    let (frames, for_picture) = std_mpsc::sync_channel(MOST_FRAMES_WAITING);
    let measure = args.measure_demo_marks;
    // For measuring: the viewer's clock at another speed (`--clock-skew-ppm`).
    sound::CLOCK_SKEW_PPM.store(args.clock_skew_ppm, std::sync::atomic::Ordering::Relaxed);
    let stop_picture = Arc::new(AtomicBool::new(false));
    let picture = Picture {
        frames: for_picture,
        screen: screen(args, ui)?,
        decoder: VideoDecoder::new()?,
        began,
        measure_marks: measure,
        stop: Arc::clone(&stop_picture),
    };
    // WHY its result comes over a channel and the thread is not joined: a
    // terminal that does not take what is written holds the thread, and
    // the viewer has to end all the same.
    let (drew, seen) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = drew.send(picture::run(picture));
    });
    let mut sound = Sound::open(Arc::clone(&ui.volume), began, args.sound)?;
    ui.show();

    // WHY the lead of the sound output is added to the buffer, for the
    // picture as for the sound: the output asks for a sample that long
    // before it is due, so without it that much of `--buffer-ms` is gone
    // before a packet may be late at all (24 of 50 ms on the machine this
    // was written on). Added to both, the two stay together. WHY outside
    // the clock: the buffer the clock has is then the watching page's, by
    // the same rule, and the lead is this machine's.
    let lead_us = sound.lead_us().await;
    let buffer_us = args.buffer_ms.saturating_mul(1000);
    let mut clock = if args.buffer_auto {
        Clock::automatic(buffer_us, AUTO_MOST_US)
    } else {
        Clock::new(buffer_us)
    };
    let mut ticks = tokio::time::interval(BUFFER_TICK);
    let mut played = Played {
        why: "",
        lasted: Duration::ZERO,
        seen: Seen::default(),
        heard: sound.heard(),
        sound_marks_us: Vec::new(),
        audio_over_least_us: Recent::new(MOST_COUNTED),
        video_over_least_us: Recent::new(MOST_COUNTED),
        buffer_us: (buffer_us, buffer_us, buffer_us),
        no_timestamp: 0,
        not_of_the_clock: 0,
        frames_dropped_full: 0,
        streams_reset: 0,
        output_lead_us: lead_us,
    };
    let mut ended = Ended::Again;
    let mut failed = None;
    played.why = loop {
        tokio::select! {
            _ = ticks.tick() => {
                clock.tick(micros_since(began, Instant::now()));
                let now = clock.buffer_us();
                played.buffer_us = (played.buffer_us.0.min(now), played.buffer_us.1.max(now), now);
            }
            asked = ui.asked() => {
                let Asked::Say(text) = asked else {
                    ended = Ended::Quit;
                    break "the viewer was ended";
                };
                if let Some(joined) = room.as_mut() {
                    ui.note(&format!("SAID {}", room::clean(&text, room::LINE_LIMIT)));
                    // A room that does not take the line is gone; the
                    // broadcast is watched on without it.
                    if !matches!(tokio::time::timeout(ROOM_WAIT, joined.say(&text)).await, Ok(Ok(()))) {
                        ui.note("ROOM_LOST it did not take a line for the chat");
                        *room = None;
                    }
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
            event = async {
                match audio.as_mut() {
                    Some(audio) => audio.reader.next_event().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(ended) = audio.take() else { continue };
                let how = match event {
                    Ok(RequestStreamEvent::Message(message)) => {
                        let line = format!("{message:?}");
                        session.on_request_message(ended.request, message)?;
                        line
                    }
                    // Without PUBLISH_DONE the session still has the
                    // subscription, which would stand in the way of the
                    // next SUBSCRIBE for the track.
                    Ok(RequestStreamEvent::Closed(closed)) => {
                        session.cancel(ended.request);
                        format!("closed {closed:?}")
                    }
                    Err(error) => {
                        session.cancel(ended.request);
                        format!("{error}")
                    }
                };
                let pause = again.after_end();
                ui.note(&format!("AUDIO_ENDED {how}; asking again in {pause} ms"));
                ask_at = Some(tokio::time::Instant::now() + Duration::from_millis(pause));
            }
            () = async {
                match cut_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                cut_at = None;
                if let Some(cut) = audio.take() {
                    session.cancel(cut.request);
                    // Dropping it resets the request stream (`cancel` above
                    // is the session's half).
                    cut.cancel();
                    let pause = again.after_end();
                    ui.note(&format!("AUDIO_ENDED cut; asking again in {pause} ms"));
                    ask_at = Some(tokio::time::Instant::now() + Duration::from_millis(pause));
                }
            }
            () = async {
                match ask_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                ask_at = None;
                let mut requests = Requests {
                    connection,
                    session: &mut session,
                    namespace: &namespace,
                    token: place.token.as_deref(),
                    answer_wait_ms: args.answer_wait_ms,
                    open_wait_ms: args.open_wait_ms,
                    rendezvous_ms: args.rendezvous_ms,
                };
                match requests.ask(&audio_name, None) {
                    Ok(asked) => {
                        let connection = Arc::clone(connection);
                        let open_wait_ms = args.open_wait_ms;
                        asking = Some(Box::pin(async move {
                            let sent = session::send_subscribe(&connection, open_wait_ms, &asked).await;
                            (asked, sent)
                        }));
                    }
                    Err(error) => ui.note(&format!("AUDIO_GIVEN_UP {error:#}")),
                }
            }
            (asked, sent) = async {
                match asking.as_mut() {
                    Some(answer) => answer.await,
                    None => std::future::pending().await,
                }
            } => {
                asking = None;
                match session::settle(&mut session, &asked, sent) {
                    Ok(subscription) => {
                        again.subscribed();
                        ui.note(&format!("AUDIO_AGAIN track_alias={}", subscription.track_alias));
                        audio = Some(subscription);
                    }
                    Err(NotSubscribed::Refused(error)) => {
                        let pause = again.after_refusal(error.retry_wait_ms());
                        ui.note(&format!(
                            "AUDIO_REFUSED error_code={:#x}; asking again in {pause} ms",
                            error.error_code
                        ));
                        ask_at = Some(tokio::time::Instant::now() + Duration::from_millis(pause));
                    }
                    // As the page: not asked for again on this session.
                    Err(NotSubscribed::Failed(error)) => {
                        ui.note(&format!("AUDIO_GIVEN_UP {error:#}"));
                    }
                }
            }
            event = events.recv() => match event {
                Some(UniEvent::Object { header, object, .. }) => {
                    // An Object of the catalog's track, which stays
                    // subscribed: a catalog that comes again is not read.
                    let of_video = header.track_alias == video.track_alias;
                    if object.status != ObjectStatus::Normal
                        || !(of_video
                            || audio
                                .as_ref()
                                .is_some_and(|audio| audio.track_alias == header.track_alias))
                    {
                        continue;
                    }
                    let Some(timestamp) = timestamp_of(&object) else {
                        played.no_timestamp += 1;
                        continue;
                    };
                    *began_playing = true;
                    let Some(base) = clock.base(micros_since(began, Instant::now()), timestamp)
                    else {
                        played.not_of_the_clock += 1;
                        continue;
                    };
                    let over_least = i64::try_from(base.over_least_us).unwrap_or(i64::MAX);
                    let after_us = |buffer_us: u64| {
                        base.at_us.saturating_add(buffer_us).saturating_add(lead_us)
                    };
                    if of_video {
                        played.video_over_least_us.push(over_least);
                        let annex_b = loc::to_annex_b(&object.payload)
                            .map_err(|error| anyhow!("a video payload: {}", error.0))?;
                        let frame = Frame {
                            group: header.group_id,
                            object: object.object_id,
                            annex_b,
                            due: began + Duration::from_micros(after_us(clock.buffer_us())),
                        };
                        // Full: the frame is left out, and the picture
                        // goes on at the next Group.
                        if frames.try_send(frame).is_err() {
                            played.frames_dropped_full += 1;
                        }
                    } else {
                        played.audio_over_least_us.push(over_least);
                        let due_us = after_us(clock.audio_buffer_us());
                        if sound.take(&object.payload, due_us)
                            && measure
                            && played.sound_marks_us.len() < MOST_COUNTED
                        {
                            played.sound_marks_us.push(due_us);
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
                // The stream of the catalog's FETCH, where the catalog came
                // on its subscription first (moqlivemock's publisher sends
                // it both ways): it is not read, and letting go of its body
                // stops it.
                Some(UniEvent::FetchOpened { stream, header }) => {
                    session.on_fetch_stream(header.request_id)?;
                    drop(stops.take_fetch_body(stream));
                }
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
    stop_picture.store(true, Ordering::Relaxed);
    drop(frames);
    played.seen = match tokio::time::timeout(PICTURE_WAIT, seen).await {
        Ok(Ok(seen)) => seen,
        Ok(Err(_)) => bail!("the picture's thread panicked"),
        // The terminal holds it: what it counted is not known.
        Err(_) => Seen::default(),
    };
    played.heard = sound.heard();

    let line = played.line(args, ui);
    say(&line);
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
            say(&error.to_string());
            eprintln!("{}", xmoq_watch::args::USAGE);
            return ExitCode::from(2);
        }
    };
    match watch(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            say(&format!("Error: {error:#}"));
            ExitCode::FAILURE
        }
    }
}
