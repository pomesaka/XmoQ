//! A viewer for the terminal: subscribes to a video track and an audio
//! track on one session, and plays both by the Timestamps of LOC: the
//! picture with the Kitty graphics protocol, the sound on the machine's
//! output. With a room, the room says where the broadcast is and gives a
//! token, its chat is written under the picture, and the viewer follows a
//! broadcast that ends and begins again. `xmoq_watch::args::USAGE` has the
//! flags and the keys.
//!
//! What it decides is in the library of this crate and in `xmoq-viewer`,
//! without input or output; the files here do the input and output, each
//! one kind of it, and `xmoq-viewer-io` does what `xmoq-agent` does as well
//! (the room, the requests, the chat on tracks).

mod audio_output;
mod picture;
mod sound;
mod terminal;

use std::collections::HashMap;
use std::future::Future;
use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_chat_tracks::names::Person;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::ObjectStatus;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{
    ControlEvent, FetchStreamOpened, MalformedRequest, PeerRequestId, RequestEvent, Role, Session,
    Transport,
};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::{self, WtConnection};
use xmoq_moqt_io::{
    Connection, RequestStreamEvent, StreamConfig, StreamEnd, StreamStops, UniEvent,
    read_request_streams, read_uni_streams,
};
use xmoq_picture::video_decoder::VideoDecoder;
use xmoq_viewer::chat_track::{shown, shown_gap};
use xmoq_viewer::reconnect::{After, BETWEEN_TRIES_MS, Ended, NextSession};
use xmoq_viewer::room::{self, Broadcast};
use xmoq_viewer_io::chat_tracks::{ChatTracks, Out as ChatOut};
use xmoq_viewer_io::ending::{self, UntilClosed};
use xmoq_viewer_io::kept::{Asking, KeptTrack, Said, Wanted};
use xmoq_viewer_io::reconnect::go_away_behind;
use xmoq_viewer_io::room_socket::{ChatTokens, ROOM_COMES_WITHIN, RoomSocket};
use xmoq_viewer_io::session::{
    self, Events, GoingAway, GoneAway, Requests, Subscription, new_url, timestamp_of,
};
use xmoq_watch::args::{Args, Chat, Fit, Source};
use xmoq_watch::clock::{AUTO_MOST_US, Clock};
use xmoq_watch::keys::{Asked, Keys};
use xmoq_watch::marks::Recent;
use xmoq_watch::{marks, tracks};

use crate::picture::{
    Frame, LINES, MOST_COUNTED, MOST_FRAMES_WAITING, Picture, Screen, Seen, Under,
};
use crate::sound::{Sound, Volume, micros_since};

/// How often the automatic buffer is moved: twice a second, as the
/// watching page's (`packages/media/src/core/playout-clock.ts`).
const BUFFER_TICK: Duration = Duration::from_millis(500);

/// More than the waits for the relay at the beginning of a broadcast come
/// to, each of which has its own time: the SETUP (10 s), the catalog (the
/// answer to its SUBSCRIBE, 10 s, and the catalog itself, 5 s), and the
/// answers to the SUBSCRIBEs of the video and of the sound (10 s each).
const BEGINNING_AT_MOST: Duration = Duration::from_secs(60);

/// How long the thread of the picture gets to end. **Provisional**: it
/// hears of the end within 20 ms unless the terminal does not take what it
/// writes.
const PICTURE_WAIT: Duration = Duration::from_secs(2);
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
    /// The number of the newest line of the chat on tracks shown, kept
    /// from one session to the next, so that a session that comes after
    /// another fetches what was missed (`lastHad` of `@xmoq/chat`).
    chat_had: Option<u64>,
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

/// Where one broadcast is played from.
struct Place {
    url: String,
    cert_sha256: [u8; 32],
    /// The fields of the Track Namespace.
    namespace: Vec<String>,
    token: Option<Vec<u8>>,
    /// The tokens of the room for the chat on tracks, where it gave them.
    chat_tokens: ChatTokens,
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
            heard = room.next() => {
                heard?;
            }
            asked = ui.asked() => match asked {
                // The chat is on the broadcast's session, which there is
                // none of yet.
                Asked::Say(text) => ui.note(&format!(
                    "SAID nothing: no broadcast yet ({})",
                    room::clean(&text, room::LINE_LIMIT)
                )),
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
                chat_tokens: ChatTokens::default(),
                broadcast: None,
            }));
        }
        (Source::Room { ask_token, .. }, Some(room)) => (room, *ask_token),
        // WHY joined again and not given up: a room that went away during
        // a session (`play`) may be back.
        (
            Source::Room {
                address,
                name,
                ask_token,
            },
            room @ None,
        ) => {
            let Some(joined) = join_room(args, address, name, ui).await? else {
                return Ok(None);
            };
            (room.insert(joined), *ask_token)
        }
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
                Duration::from_millis(args.room_wait_ms),
                room.token(room::TokenRole::Sub, &broadcast.namespace),
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
    let mut chat_tokens = ChatTokens::default();
    if let (true, Some(chat)) = (ask_token, &args.chat) {
        let Some(tokens) = ui
            .unless_ended(
                "the room's tokens for the chat",
                Duration::from_millis(args.room_wait_ms),
                room.chat_tokens(&person_of(chat)),
            )
            .await?
        else {
            return Ok(None);
        };
        chat_tokens = tokens;
    }
    Ok(Some(Place {
        url: broadcast.relay_url.clone(),
        cert_sha256: room::certificate_hash(&broadcast.cert_sha256_hex)?,
        namespace: broadcast.namespace.clone(),
        token,
        chat_tokens,
        broadcast: Some(broadcast),
    }))
}

/// Who the viewer is in the chat on tracks.
fn person_of(chat: &Chat) -> Person {
    Person {
        room: chat.room.clone(),
        name: chat.name.clone(),
    }
}

async fn run(args: &Args, ui: &mut Ui) -> Result<()> {
    // With a room: where the broadcast is, and a token, are the room's to say.
    let mut room = None;
    if let Source::Room { address, name, .. } = &args.source {
        let Some(joined) = join_room(args, address, name, ui).await? else {
            return Ok(());
        };
        room = Some(joined);
    }
    // WHY the first failure ends the viewer, with a room as well: a person
    // started it and looks at it, and a relay that turns it down is to be
    // seen at once.
    let mut rule = NextSession::new(BETWEEN_TRIES_MS, room.is_some(), true);
    let since = Instant::now();
    let now_ms = || u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
    loop {
        let Some(mut place) = place(args, &mut room, ui).await? else {
            return Ok(());
        };
        // WHY the same certificate hash where a GOAWAY names another URL:
        // the viewer has no other for it; a relay elsewhere with another
        // certificate turns the connection down, and the viewer goes on as
        // after any failure.
        let next = rule.next(&place.url, now_ms());
        place.url = next.url;
        if next.wait_ms > 0 {
            let wait = Duration::from_millis(next.wait_ms);
            let waiting = async {
                tokio::time::sleep(wait).await;
                Ok(())
            };
            if ui
                .unless_ended("the wait", wait * 2, waiting)
                .await?
                .is_none()
            {
                return Ok(());
            }
        }
        let mut began = false;
        let streams = StreamConfig::default();
        let connecting = async { Ok(wt::connect(&place.url, place.cert_sha256, &streams).await?) };
        let connect_wait = Duration::from_millis(args.connect_wait_ms);
        let played = match ui.unless_ended("the relay", connect_wait, connecting).await {
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
                ending::close(&connection, &played).await;
                played
            }
            Err(error) => Err(error),
        };
        if let Ok(Ended::Moved(url)) = &played {
            ui.note(&format!(
                "MOVED {}",
                url.as_deref().unwrap_or("to the same URL")
            ));
        }
        let end = match &played {
            Ok(end) => end.clone(),
            Err(_) => Ended::Failed { began },
        };
        match (rule.ended(end), played) {
            (After::Connect, Err(error)) => ui.note(&format!("PLAY_ENDED {error:#}")),
            (After::Connect, Ok(_)) => {}
            (After::End, _) => return Ok(()),
            (After::EndWithError, played) => return played.map(|_| ()),
        }
    }
}

/// The room at `address`, which may begin listening only later
/// (`ROOM_COMES_WITHIN`); `None` where the viewer is to end first.
async fn join_room(
    args: &Args,
    address: &str,
    name: &str,
    ui: &mut Ui,
) -> Result<Option<RoomSocket>> {
    let joining = RoomSocket::join_when_there(address, name, ROOM_COMES_WITHIN);
    // The last try may begin as the refusals' time ends.
    let wait = ROOM_COMES_WITHIN + Duration::from_millis(args.room_wait_ms);
    ui.unless_ended("the room", wait, joining).await
}

/// The subscriptions of a broadcast.
struct Subscribed {
    video: Subscription,
    audio: Subscription,
    /// The name of the sound's track, to ask for it again.
    audio_name: String,
    /// What keeps the catalog's requests open.
    catalog: Option<session::CatalogKept>,
    /// What the catalog had, as a line for the log; `None` where the tracks
    /// were named.
    found: Option<String>,
}

/// The tracks to play: by name, or the first of each kind the catalog has.
async fn subscriptions(
    args: &Args,
    requests: &mut Requests<'_>,
    events: &mut Events,
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
        video: events
            .while_waiting(requests.subscribe(&video, args.video_timeout_ms))
            .await??,
        audio: events
            .while_waiting(requests.subscribe(&audio, None))
            .await??,
        audio_name: audio,
        catalog: kept,
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

/// Shows or writes what came of the chat on tracks.
fn tell_chat(ui: &mut Ui, out: ChatOut) {
    match out {
        ChatOut::Line(line) => ui.chat(shown(line.number, &line.name, &line.text)),
        ChatOut::Gap(gap) => {
            ui.note(&format!(
                "CHAT_GAP after={} before={}",
                gap.after, gap.before
            ));
            ui.chat(shown_gap(gap));
        }
        ChatOut::Note(line) => ui.note(&line),
    }
}

/// Writes what became of the sound kept on the session.
fn tell_audio(ui: &mut Ui, said: Said) {
    let line = match said {
        Said::Subscribed { track_alias } => format!("AUDIO_AGAIN track_alias={track_alias}"),
        Said::Ended { how, pause_ms } => {
            format!("AUDIO_ENDED {how}; asking again in {pause_ms} ms")
        }
        Said::Refused {
            error_code,
            pause_ms,
        } => format!("AUDIO_REFUSED error_code={error_code:#x}; asking again in {pause_ms} ms"),
        // As the page: not asked for again on this session.
        Said::GivenUp(why) | Said::Failed { why, .. } => format!("AUDIO_GIVEN_UP {why}"),
        Said::Other(other) => format!("AUDIO_SAID {other:?}"),
    };
    ui.note(&line);
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
    kept: &mut UntilClosed,
) -> Result<Ended> {
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let stops = StreamStops::default();
    let (queue, received) = mpsc::channel(64);
    let mut events = Events::new(received);
    kept.readers = Some(read_uni_streams(
        Arc::clone(connection),
        StreamConfig::default(),
        stops.clone(),
        queue,
        |event| event,
    ));
    // The requests the peer opens: none is served, and each is refused
    // (`session::refuse_peer_request`). They are read from here on and
    // answered once the broadcast plays; until then they wait in the queue.
    let (asks, mut asked) = mpsc::channel(16);
    kept.requests = Some(read_request_streams(
        Arc::clone(connection),
        StreamConfig::default(),
        stops.clone(),
        asks,
        |event| event,
    ));
    let mut refused: HashMap<u64, PeerRequestId> = HashMap::new();
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
        let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
        // 10 seconds, as `@xmoq/moqt` gives the peer's SETUP.
        session::setup(
            connection,
            &mut session,
            &mut events,
            control_kept,
            place.token.as_deref(),
            implementation,
            args.setup_wait_ms,
        )
        .await?;
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
    let beginning_ended = ui
        // Each of the three SUBSCRIBEs may be held that much longer.
        .unless_ended(
            "the relay's answers",
            BEGINNING_AT_MOST + Duration::from_millis(args.rendezvous_ms.unwrap_or(0)) * 3,
            beginning,
        )
        .await;
    let subscribed = match beginning_ended {
        Ok(Some(subscribed)) => subscribed,
        Ok(None) => return Ok(Ended::Quit),
        Err(error) => {
            if let Some(GoneAway(go_away)) = error.chain().find_map(|cause| cause.downcast_ref()) {
                ui.note(&format!("GOAWAY timeout_ms={}", go_away.timeout_ms));
                return Ok(Ended::Moved(new_url(go_away)));
            }
            if error.chain().any(<dyn std::error::Error>::is::<GoingAway>)
                && let Some(go_away) =
                    go_away_behind(&mut session, &mut events, args.answer_wait_ms).await?
            {
                ui.note(&format!(
                    "GOAWAY timeout_ms={} after GOING_AWAY",
                    go_away.timeout_ms
                ));
                return Ok(Ended::Moved(new_url(&go_away)));
            }
            return Err(error);
        }
    };
    let Subscribed {
        mut video,
        audio,
        audio_name,
        mut catalog,
        found,
    } = subscribed;
    // WHY the sound is asked for again on this session, and the picture
    // goes on: a relay may end the sound's subscription alone (it ends the
    // one that falls behind), and a new session would stop the picture for
    // the sake of the sound. The watching page does the same
    // (`keepTrackOnSession` of `@xmoq/media`), by the same rule
    // (`xmoq_viewer_io::kept`).
    let mut audio = KeptTrack::subscribed(
        Wanted {
            namespace: namespace.clone(),
            name: audio_name,
            rendezvous_ms: args.rendezvous_ms,
            payload_most: None,
            again_after_failure: false,
        },
        Asking {
            connection: Arc::clone(connection),
            stops: stops.clone(),
            token: place.token.clone(),
            answer_wait_ms: args.answer_wait_ms,
            open_wait_ms: args.open_wait_ms,
        },
        audio,
    );
    let mut cut_at = args
        .cut_sound_s
        .map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds));
    if let Some(line) = found {
        ui.note(&line);
    }
    // The chat on tracks, beside the room's WebSocket: what fails of
    // it is said, and the broadcast plays on without it.
    let mut chat = match &args.chat {
        Some(chat) => {
            let begun = async {
                let requests = Requests {
                    connection,
                    session: &mut session,
                    namespace: &namespace,
                    token: place.chat_tokens.subscribe.as_deref(),
                    answer_wait_ms: args.answer_wait_ms,
                    open_wait_ms: args.open_wait_ms,
                    rendezvous_ms: None,
                };
                let publish = place.chat_tokens.publish.as_deref();
                ChatTracks::begin(
                    requests,
                    publish,
                    connection,
                    &stops,
                    &person_of(chat),
                    chat.catch_up,
                    ui.chat_had,
                )
                .await
            };
            match begun.await {
                Ok(mut tracks) => {
                    ui.note(&format!(
                        "CHAT_TRACKS room={} name={}",
                        room::clean(&chat.room, room::LINE_LIMIT),
                        room::clean(&chat.name, room::LINE_LIMIT)
                    ));
                    if let Some(text) = &chat.say {
                        let taken = tracks.say(connection, text.clone());
                        ui.note(&format!(
                            "SAID_ON_TRACK {} {}",
                            if taken { "sent" } else { "not-sent" },
                            room::clean(text, room::LINE_LIMIT)
                        ));
                    }
                    Some(tracks)
                }
                Err(error) => {
                    ui.note(&format!("CHAT_TRACKS_FAILED {error:#}"));
                    None
                }
            }
        }
        None => None,
    };

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
    let mut video_malformed = false;
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
                if let Some(tracks) = chat.as_mut() {
                    let taken = tracks.say(connection, text.clone());
                    ui.note(&format!(
                        "SAID_ON_TRACK {} {}",
                        if taken { "sent" } else { "not-sent" },
                        room::clean(&text, room::LINE_LIMIT)
                    ));
                } else {
                    ui.note("SAID nothing: no chat");
                }
            }
            heard = async {
                match room.as_mut() {
                    Some(room) => room.next().await,
                    None => std::future::pending().await,
                }
            } => match heard {
                Ok(broadcast) => {
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
                        // §10.4: "re-issue that specific request on a session
                        // at the specified URI (or the current session if no
                        // URI is provided)". WHY a new session for the video
                        // also without a URI: the picture's clock and its first
                        // frame begin with a session here, and the sound
                        // follows the picture.
                        if let RequestEvent::GoAway(go_away) =
                            session.on_request_message(video.request, message)?
                        {
                            ui.note(&format!("REQUEST_GOAWAY video timeout_ms={}", go_away.timeout_ms));
                            ended = Ended::Moved(new_url(&go_away));
                            break "the relay sent GOAWAY for the video";
                        }
                    }
                    Ok(RequestStreamEvent::Closed(_)) => {}
                    Err(error) => failed = Some(error.into()),
                }
                break "the video track ended";
            }
            came = audio.next() => {
                if let Some(said) = audio.on(&mut session, came)? {
                    tell_audio(ui, said);
                }
            }
            () = async {
                match cut_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                cut_at = None;
                if let Some(said) = audio.cut(&mut session) {
                    tell_audio(ui, said);
                }
            }
            Some(event) = asked.recv() => {
                let event = match chat.as_mut() {
                    Some(tracks) => tracks.on_peer_request(&mut session, connection, event).await?,
                    None => Some(event),
                };
                if let Some(event) = event {
                    session::refuse_peer_request(&mut session, &mut refused, event).await?;
                }
            }
            came = async {
                match chat.as_mut() {
                    Some(tracks) => tracks.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(tracks) = chat.as_mut() else { continue };
                for out in tracks.on(&mut session, came).await? {
                    tell_chat(ui, out);
                }
                ui.chat_had = tracks.last_shown();
            }
            event = events.recv_sorted(&session, &stops) => match event {
                Some(UniEvent::Object { header, object, .. })
                    if chat.as_ref().and_then(ChatTracks::lines_alias) == Some(header.track_alias) =>
                {
                    if let Some(line) = chat.as_mut().and_then(|tracks| tracks.live(header.group_id, &object.payload)) {
                        ui.chat(shown(line.number, &line.name, &line.text));
                    }
                    ui.chat_had = chat.as_ref().and_then(ChatTracks::last_shown);
                }
                Some(UniEvent::Object { header, object, .. }) => {
                    // An Object of the catalog's track, which stays
                    // subscribed: a catalog that comes again is not read.
                    let of_video = header.track_alias == video.track_alias;
                    if object.status != ObjectStatus::Normal
                        || !(of_video
                            || audio.track_alias() == Some(header.track_alias))
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
                Some(UniEvent::FetchOpened { stream, header })
                    if chat.as_ref().and_then(ChatTracks::fetch_request_id) == Some(header.request_id) =>
                {
                    session.on_fetch_stream(header.request_id)?;
                    if let Some(tracks) = chat.as_mut() {
                        tracks.read_fetch(stream, header);
                    }
                }
                Some(UniEvent::FetchOpened { stream, header }) => {
                    drop(stops.take_fetch_body(stream));
                    // Its stream is let go of: the FETCH is over once
                    // it was answered too.
                    if let FetchStreamOpened::Fetch(request) = session.on_fetch_stream(header.request_id)? {
                        session.on_fetch_stream_closed(request);
                    }
                }
                Some(UniEvent::Failed(error)) => {
                    failed = Some(error.into());
                    break "the session failed";
                }
                // WHY the subscriptions end at once, where §3.6 recommends
                // waiting until none is Established: the picture would stand
                // still while the old session ends anyway, and the new one is
                // opened as soon as this one is closed with NO_ERROR. §10.4:
                // "A subscriber SHOULD individually UNSUBSCRIBE for each
                // existing subscription", which leaving `play` does.
                Some(UniEvent::Control(message)) => {
                    if let ControlEvent::GoAway(go_away) = session.on_control_message(message)? {
                        ui.note(&format!("GOAWAY timeout_ms={}", go_away.timeout_ms));
                        ended = Ended::Moved(new_url(&go_away));
                        break "the relay sent GOAWAY";
                    }
                }
                // §2.4.2: "it MUST cancel any corresponding subscription or
                // fetches for that Track". WHY the session ends for the
                // video alone, and a new one is made at the same place
                // (`Ended::Broken`): the picture's clock and its first frame
                // begin with a session, as after a GOAWAY for the video.
                Some(UniEvent::TrackMalformed { header, error, .. }) => {
                    ui.note(&format!("TRACK_MALFORMED track_alias={} error={error}", header.track_alias));
                    events.stop_streams_of(header.track_alias, &stops);
                    for cancelled in session.cancel_malformed(header.track_alias) {
                        // The viewer takes no PUBLISH of the relay.
                        let (MalformedRequest::Subscribe(request) | MalformedRequest::Fetch(request)) = cancelled else {
                            continue;
                        };
                        if request == video.request {
                            video_malformed = true;
                        } else if let Some(said) = audio.malformed(request) {
                            tell_audio(ui, said);
                        } else if catalog.as_mut().is_some_and(|catalog| catalog.malformed(request)) {
                            ui.note("CATALOG_MALFORMED");
                        } else if let Some(outs) = chat.as_mut().and_then(|tracks| tracks.malformed(request)) {
                            for out in outs {
                                tell_chat(ui, out);
                            }
                        } else {
                            ui.note(&format!("TRACK_MALFORMED_UNCLAIMED request_id={}", request.value()));
                        }
                    }
                    if video_malformed {
                        ended = Ended::Broken;
                        break "the video track is malformed";
                    }
                }
                Some(other) => bail!("not expected of the relay: {other:?}"),
                None => {
                    failed = Some(anyhow!("the connection is gone"));
                    break "the connection is gone";
                }
            },
        }
    };
    if video_malformed {
        video.cancel_with(stream_reset_code::MALFORMED_TRACK);
    }
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
    let stop = ending::on_signals();
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
        chat_had: None,
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
