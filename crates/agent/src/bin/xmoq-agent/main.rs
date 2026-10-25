//! A reader of a room for an agent: joins the room as `--name`, connects to
//! the broadcast the room names, and writes what happens in its chat as
//! lines of JSON on stdout and in `<DIR>/events.jsonl`; a line of stdin is
//! said in the chat. It follows a broadcast that ends and begins again.
//! `xmoq_agent::args::USAGE` has the flags.
//!
//! What it decides is in the library of this crate and in `xmoq-viewer`;
//! the files here do the input and output, with `xmoq-viewer-io`.

mod attend;
mod frames;
mod meeting;
mod output;
mod stdin;
mod video;

use std::future::Future;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use tokio::sync::{mpsc, watch};
use xmoq_agent::args::{Args, Source};
use xmoq_agent::events::{BroadcastState, Event};
use xmoq_agent::input::{self, Input};
use xmoq_agent::presence::Presence;
use xmoq_chat_tracks::names::Person;
use xmoq_moqt_io::wt;
use xmoq_moqt_io::{Connection, StreamConfig};
use xmoq_viewer::room::{self, Broadcast, Relay};
use xmoq_viewer_io::room_socket::{ChatTokens, ROOM_COMES_WITHIN, RoomSocket};
use xmoq_viewer_io::session::termination_code;

use crate::attend::{Ended, UntilClosed, attend};
use crate::frames::Pictures;

use crate::output::{Output, now_ms};

/// The directory of the pictures in `--out`, with one directory a run in
/// it.
const FRAMES_DIR: &str = "frames";

/// Where the pictures of this run go: `<out>/frames/<when it began, ms>/`.
///
/// WHY a directory a run: `Frames` names its files from 1 on each run, and
/// those of a run before in the same `--out` would be written over, and not
/// counted by `--frames-kept`. WHY NOT go on from the numbers there: what
/// names the files is `xmoq-picture`'s. WHY NOT empty the directory: the
/// pictures an agent asked for before are its own.
fn frames_dir_of(out: &str) -> std::path::PathBuf {
    Path::new(out).join(FRAMES_DIR).join(now_ms().to_string())
}
use crate::stdin::Line;

/// How long the room, a token, the relay's connection each get.
/// **Provisional**, as `xmoq-watch` has them.
const ROOM_WAIT: Duration = Duration::from_secs(10);
/// The least time from one try at a broadcast to the next. **Provisional.**
const BETWEEN_TRIES: Duration = Duration::from_secs(1);

/// What the reader shares between its parts while it runs.
pub struct Reader {
    out: Output,
    lines: mpsc::Receiver<Line>,
    /// Whether stdin may still bring lines.
    stdin_open: bool,
    stop: watch::Receiver<bool>,
    deadline: Option<tokio::time::Instant>,
    presence: Presence,
    /// The newest line of the chat written, from one session to the next.
    had: Option<u64>,
    /// The newest piece of speech written, and of which transcript, from one
    /// session to the next.
    speech_had: Option<(Vec<String>, u64)>,
    /// `--frames on`.
    frames: bool,
    /// The picture asked for, from one session to the next.
    frame: FrameAsking,
}

/// The picture asked for (`/frame`), from one session to the next, as
/// `Frames` keeps the asking (`begin_again`).
#[derive(Default)]
pub struct FrameAsking {
    /// Until when it is waited for.
    until: Option<tokio::time::Instant>,
    /// It was asked for while no video could take it (between sessions, or
    /// while one begins), and is to be asked for when one can.
    unsent: bool,
    /// It came not in time while no session could tell `Frames` to give
    /// the asking up.
    forgo_unsent: bool,
    /// In a meeting, whose picture was asked for while unsent (`/frame <id>`).
    of: Option<String>,
}

/// How long a picture that was asked for is waited for. **Provisional.**
/// WHY 10 s: the answer waits for the frames of one Group to be decoded,
/// or, before any came, for the first key frame, which the demo sends every
/// second; and while the reader connects again, for the next session's
/// video.
pub const FRAME_WAIT: Duration = Duration::from_secs(10);

/// What came from outside the session while something is waited for.
enum Outside {
    /// The reader is to end: a signal, or `--seconds`.
    End,
    /// A line of stdin.
    Line(Line),
}

impl Reader {
    /// Writes an event.
    pub fn tell(&mut self, event: &Event) {
        self.out.write(event);
    }

    /// Writes an error, which the reader goes on after.
    pub fn error(&mut self, message: String) {
        self.tell(&Event::Error { message });
    }

    /// The next thing from outside the session.
    async fn outside(&mut self) -> Outside {
        let deadline = self.deadline;
        loop {
            let stdin_open = self.stdin_open;
            let frame_asked_until = self.frame.until;
            let mut frame_due = false;
            tokio::select! {
                () = async {
                    match frame_asked_until {
                        Some(until) => tokio::time::sleep_until(until).await,
                        None => std::future::pending().await,
                    }
                } => frame_due = true,
                changed = self.stop.wait_for(|stop| *stop) => {
                    // A sender that is gone has nothing more to say.
                    let _ = changed;
                    return Outside::End;
                }
                () = async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                } => return Outside::End,
                line = self.lines.recv(), if stdin_open => match line {
                    Some(line) => return Outside::Line(line),
                    None => self.stdin_open = false,
                },
            }
            if frame_due {
                self.frame_not_in_time();
            }
        }
    }

    /// The picture asked for came not in time: said, and the asking given up.
    fn frame_not_in_time(&mut self) {
        self.frame.until = None;
        self.error(format!(
            "frame: no picture within {} s",
            FRAME_WAIT.as_secs()
        ));
        if self.frame.unsent {
            self.frame.unsent = false;
        } else {
            self.frame.forgo_unsent = true;
        }
    }

    /// `/frame`: waited for from now, where it is not already.
    pub fn frame_asked(&mut self) {
        self.frame
            .until
            .get_or_insert_with(|| tokio::time::Instant::now() + FRAME_WAIT);
    }

    /// A line of stdin while there is no chat to say it in.
    fn line_without_chat(&mut self, line: Line, why: &str) {
        match line {
            Line::TooLong(length) => self.too_long(length),
            Line::Read(text) => match input::read(&text) {
                Input::Nothing => {}
                Input::Say(_) => self.error(format!("said nothing: {why}")),
                // WHY kept for the next video and not refused: an agent
                // that asks while the reader connects again (a GOAWAY, a
                // broadcast that came) would otherwise have to ask again.
                Input::Frame(of) if self.frames => {
                    self.frame_asked();
                    self.frame.unsent = true;
                    self.frame.of = of;
                }
                Input::Frame(_) => self.error("frame: --frames off".to_string()),
                Input::Unknown(name) => self.unknown_command(&name),
            },
        }
    }

    fn too_long(&mut self, length: usize) {
        self.error(format!(
            "said nothing: a line of {length} bytes, more than {}",
            stdin::MOST_A_LINE
        ));
    }

    fn unknown_command(&mut self, name: &str) {
        self.error(format!(
            "no command /{name}: /frame is one, and // begins a line that is said"
        ));
    }

    /// Waits for `doing`, at most `wait`; `None` where the reader is to end
    /// first. Lines of stdin meanwhile are told as not said.
    async fn unless_ended<T>(
        &mut self,
        what: &str,
        wait: Duration,
        doing: impl Future<Output = Result<T>>,
    ) -> Result<Option<T>> {
        let mut doing = Box::pin(tokio::time::timeout(wait, doing));
        loop {
            tokio::select! {
                done = &mut doing => {
                    let Ok(done) = done else {
                        bail!("{what}: nothing within {} ms", wait.as_millis());
                    };
                    return done.map(Some);
                }
                outside = self.outside() => match outside {
                    Outside::End => return Ok(None),
                    Outside::Line(line) => self.line_without_chat(line, &format!("waiting for {what}")),
                },
            }
        }
    }
}

/// `sub` in `xmoq/transcript/<prefix>`, of the room, as an AUTHORIZATION
/// TOKEN; `Some(None)` where the room gives none, `None` where the reader is
/// to end first.
async fn transcript_token_of(
    room: &mut RoomSocket,
    reader: &mut Reader,
    prefix: &[String],
) -> Result<Option<Option<Vec<u8>>>> {
    let fields: Vec<String> = xmoq_transcript_tracks::names::PREFIX
        .iter()
        .map(|field| (*field).to_string())
        .chain(prefix.iter().cloned())
        .collect();
    let given = reader
        .unless_ended(
            "the room's token for the transcript",
            ROOM_WAIT,
            room.token(room::TokenRole::Sub, &fields),
        )
        .await?;
    Ok(given.map(|given| given.map(|text| room::authorization_token(&text))))
}

/// Where one session goes: a broadcast, or the room's relay for its chat.
pub struct Place {
    url: String,
    cert_sha256: [u8; 32],
    /// The broadcast's namespace; `None` for a session for the chat alone.
    namespace: Option<Vec<String>>,
    token: Option<Vec<u8>>,
    chat_tokens: ChatTokens,
    /// The broadcast of the room this is, where a room said it.
    broadcast: Option<Broadcast>,
    /// The prefix whose transcript is followed; `None` with `--speech off`.
    transcript: Option<Vec<String>>,
    /// `sub` in `xmoq/transcript/<prefix>`, where the room gave one.
    transcript_token: Option<Vec<u8>>,
}

/// The prefix whose transcript is followed (`xmoq_agent::transcript`), with
/// what the room's hello said and the broadcast's namespace, where there
/// are; none with `--speech off`.
fn transcript_of(
    args: &Args,
    hello: Option<&[String]>,
    broadcast: Option<&[String]>,
) -> Option<Vec<String>> {
    if !args.speech {
        return None;
    }
    let meeting = args.meeting.as_ref().map(|(room, _)| room.as_str());
    xmoq_agent::transcript::prefix(args.transcript.as_deref(), hello, meeting, broadcast)
}

/// Where the reader connects in a room.
enum Found {
    /// The room's broadcast, at its relay.
    Broadcast(Broadcast),
    /// The relay of the rooms, for the chat, while the room has no
    /// broadcast.
    Relay(Relay),
}

/// Waits until the room has a broadcast, or names its relay. `None` when
/// the reader is to end first.
async fn broadcast_of(room: &mut RoomSocket, reader: &mut Reader) -> Result<Option<Found>> {
    loop {
        if let Some(broadcast) = room.current() {
            return Ok(Some(Found::Broadcast(broadcast)));
        }
        // WHY at once, without a broadcast: the chat is on the relay, and
        // the reader takes part in it before anybody broadcasts. The session
        // ends when a broadcast comes (`attend`), for one at its relay.
        if let Some(relay) = room.relay() {
            return Ok(Some(Found::Relay(relay)));
        }
        tokio::select! {
            heard = room.next() => {
                heard?;
            }
            outside = reader.outside() => match outside {
                Outside::End => return Ok(None),
                // The chat is on the broadcast's session, which there is
                // none of yet.
                Outside::Line(line) => reader.line_without_chat(line, "no broadcast yet"),
            },
        }
    }
}

/// Where the broadcast is now, with the tokens of the room. `None` when
/// the reader is to end before a room has one.
async fn place(
    args: &Args,
    room: &mut Option<RoomSocket>,
    reader: &mut Reader,
) -> Result<Option<Place>> {
    let (room, ask_token) = match (&args.source, room) {
        (
            Source::Relay {
                url,
                cert_sha256,
                namespace,
            },
            _,
        ) => {
            // A meeting has a broadcast for each participant, and the
            // transcript of all of them under its prefix (`transcript_of`).
            let namespace = args.meeting.is_none().then(|| namespace.clone());
            return Ok(Some(Place {
                url: url.clone(),
                cert_sha256: *cert_sha256,
                transcript: transcript_of(args, None, namespace.as_deref()),
                namespace,
                token: None,
                chat_tokens: ChatTokens::default(),
                broadcast: None,
                transcript_token: None,
            }));
        }
        (Source::Room { ask_token, .. }, Some(room)) => (room, *ask_token),
        (Source::Room { .. }, None) => bail!("the room is gone"),
    };
    let Some(found) = broadcast_of(room, reader).await? else {
        return Ok(None);
    };
    let mut token = None;
    let mut chat_tokens = ChatTokens::default();
    // A session for the chat alone has no broadcast whose namespace to
    // take: it follows the transcript the room or the flag names, as in a
    // meeting, whose participants each have a broadcast of their own.
    let hello = room.transcript();
    let transcript = match &found {
        Found::Broadcast(broadcast) => {
            transcript_of(args, hello.as_deref(), Some(&broadcast.namespace))
        }
        Found::Relay(_) => transcript_of(args, hello.as_deref(), None),
    };
    let mut transcript_token = None;
    if ask_token {
        if let Found::Broadcast(broadcast) = &found {
            let Some(given) = reader
                .unless_ended(
                    "the room's token",
                    ROOM_WAIT,
                    room.token(room::TokenRole::Sub, &broadcast.namespace),
                )
                .await?
            else {
                return Ok(None);
            };
            token = given.map(|text| room::authorization_token(&text));
        }
        let Some(tokens) = reader
            .unless_ended(
                "the room's tokens for the chat",
                ROOM_WAIT,
                room.chat_tokens(&person_of(args)),
            )
            .await?
        else {
            return Ok(None);
        };
        chat_tokens = tokens;
        if let Some(prefix) = &transcript {
            let Some(given) = transcript_token_of(room, reader, prefix).await? else {
                return Ok(None);
            };
            transcript_token = given;
        }
    }
    Ok(Some(match found {
        Found::Broadcast(broadcast) => Place {
            url: broadcast.relay_url.clone(),
            cert_sha256: room::certificate_hash(&broadcast.cert_sha256_hex)?,
            namespace: Some(broadcast.namespace.clone()),
            token,
            chat_tokens,
            broadcast: Some(broadcast),
            transcript,
            transcript_token,
        },
        Found::Relay(relay) => Place {
            url: relay.relay_url,
            cert_sha256: room::certificate_hash(&relay.cert_sha256_hex)?,
            namespace: None,
            // WHY the chat's `sub` in the SETUP: a relay with
            // `--auth-key-hex` takes any token of the room there, and the
            // session's own, `sub` in the broadcast's namespace, needs a
            // broadcast.
            token: chat_tokens.subscribe.clone(),
            chat_tokens,
            broadcast: None,
            transcript,
            transcript_token,
        },
    }))
}

/// Writes that the broadcast of `place` began or ended; nothing for a
/// session for the chat alone.
fn tell_broadcast(reader: &mut Reader, place: &Place, state: BroadcastState) {
    if let Some(namespace) = &place.namespace {
        reader.tell(&Event::Broadcast {
            state,
            namespace: namespace.clone(),
        });
    }
}

/// Who the reader is in the chat.
fn person_of(args: &Args) -> Person {
    Person {
        room: args.room.clone(),
        name: args.name.clone(),
    }
}

async fn run(args: &Args, reader: &mut Reader, pictures: &mut Option<Pictures>) -> Result<()> {
    let mut room = match &args.source {
        Source::Room { address, .. } => {
            let joining = RoomSocket::join_when_there(address, &args.name, ROOM_COMES_WITHIN);
            // The last try may begin as the refusals' time ends.
            let wait = ROOM_COMES_WITHIN + ROOM_WAIT;
            let Some(room) = reader.unless_ended("the room", wait, joining).await? else {
                return Ok(());
            };
            Some(room)
        }
        Source::Relay { .. } => None,
    };
    let follows = room.is_some();
    let mut tried: Option<Instant> = None;
    // The last GOAWAY: the URL left, and where to go instead, for as long
    // as the place is the one left (as `xmoq-watch` has it, where the reasons
    // are).
    let mut moved: Option<(String, String)> = None;
    let mut just_moved = false;
    loop {
        let Some(mut place) = place(args, &mut room, reader).await? else {
            return Ok(());
        };
        match &moved {
            Some((left, to)) if *left == place.url => place.url.clone_from(to),
            Some(_) => moved = None,
            None => {}
        }
        if !just_moved
            && let Some(wait) = tried.and_then(|tried| BETWEEN_TRIES.checked_sub(tried.elapsed()))
        {
            let waiting = async {
                tokio::time::sleep(wait).await;
                Ok(())
            };
            if reader
                .unless_ended("the wait", BETWEEN_TRIES * 2, waiting)
                .await?
                .is_none()
            {
                return Ok(());
            }
        }
        just_moved = false;
        tried = Some(Instant::now());
        tell_broadcast(reader, &place, BroadcastState::Began);
        let streams = StreamConfig::default();
        let connecting = async { Ok(wt::connect(&place.url, place.cert_sha256, &streams).await?) };
        let attended = match reader
            .unless_ended("the relay", ROOM_WAIT, connecting)
            .await
        {
            Ok(None) => return Ok(()),
            Ok(Some(connection)) => {
                let connection = Arc::new(connection);
                let mut kept = UntilClosed::default();
                // Boxed: the session's future is the largest by far, and
                // the reader's own would grow by it.
                let attended = Box::pin(attend(
                    args,
                    &connection,
                    &place,
                    &mut room,
                    reader,
                    pictures,
                    &mut kept,
                ))
                .await;
                connection.close(attended.as_ref().err().map_or(0, termination_code), b"");
                let _ = tokio::time::timeout(Duration::from_millis(500), connection.closed()).await;
                attended
            }
            Err(error) => Err(error),
        };
        reader.presence.session_ended();
        tell_broadcast(reader, &place, BroadcastState::Ended);
        match attended {
            Ok(Ended::Moved(url)) => {
                let left = moved
                    .as_ref()
                    .map_or_else(|| place.url.clone(), |(left, _)| left.clone());
                moved = Some((left, url.unwrap_or_else(|| place.url.clone())));
                just_moved = true;
            }
            Ok(Ended::Again) if follows => {}
            Ok(Ended::Quit | Ended::Again) => return Ok(()),
            Err(error) if !follows => return Err(error),
            Err(error) => reader.error(format!("the session ended: {error:#}")),
        }
    }
}

async fn read(args: &Args) -> Result<()> {
    let (stopping, stop) = watch::channel(false);
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
    let mut reader = Reader {
        out: Output::open(Path::new(&args.out))?,
        lines: stdin::lines(),
        stdin_open: true,
        stop,
        deadline: args
            .seconds
            .map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds)),
        presence: Presence::new(&args.room, &args.name),
        had: None,
        speech_had: None,
        frames: args.frames,
        frame: FrameAsking::default(),
    };
    let mut pictures = if args.frames {
        Some(Pictures::start(args.pictures, frames_dir_of(&args.out))?)
    } else {
        None
    };
    let ran = run(args, &mut reader, &mut pictures).await;
    if let Err(error) = &ran {
        reader.error(format!("{error:#}"));
    }
    // On every way out of `run`, before `main` returns and the process
    // ends with the thread of the pictures.
    if let Some(pictures) = pictures {
        pictures.end().await;
    }
    ran
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match xmoq_agent::args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!("{}", xmoq_agent::args::USAGE);
            return ExitCode::from(2);
        }
    };
    match read(&args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
