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
mod output;
mod stdin;

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
use xmoq_viewer_io::room_socket::{ChatTokens, RoomSocket};
use xmoq_viewer_io::session::termination_code;

use crate::attend::{Ended, UntilClosed, attend};
use crate::frames::Pictures;

/// The directory of the pictures in `--out`.
const FRAMES_DIR: &str = "frames";
use crate::output::Output;
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
}

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
            tokio::select! {
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
        }
    }

    /// A line of stdin while there is no chat to say it in.
    fn line_without_chat(&mut self, line: Line, why: &str) {
        match line {
            Line::TooLong(length) => self.too_long(length),
            Line::Read(text) => match input::read(&text) {
                Input::Nothing => {}
                Input::Say(_) => self.error(format!("said nothing: {why}")),
                Input::Frame => self.error(format!("frame: {why}")),
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
            return Ok(Some(Place {
                url: url.clone(),
                cert_sha256: *cert_sha256,
                namespace: Some(namespace.clone()),
                token: None,
                chat_tokens: ChatTokens::default(),
                broadcast: None,
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
    }
    Ok(Some(match found {
        Found::Broadcast(broadcast) => Place {
            url: broadcast.relay_url.clone(),
            cert_sha256: room::certificate_hash(&broadcast.cert_sha256_hex)?,
            namespace: Some(broadcast.namespace.clone()),
            token,
            chat_tokens,
            broadcast: Some(broadcast),
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

async fn run(args: &Args, reader: &mut Reader) -> Result<()> {
    let mut room = match &args.source {
        Source::Room { address, .. } => {
            let joining = RoomSocket::join(address, &args.name);
            let Some(room) = reader.unless_ended("the room", ROOM_WAIT, joining).await? else {
                return Ok(());
            };
            Some(room)
        }
        Source::Relay { .. } => None,
    };
    let follows = room.is_some();
    let mut pictures = if args.frames {
        Some(Pictures::start(
            args.pictures,
            Path::new(&args.out).join(FRAMES_DIR),
        )?)
    } else {
        None
    };
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
                let attended = attend(
                    args,
                    &connection,
                    &place,
                    &mut room,
                    reader,
                    &mut pictures,
                    &mut kept,
                )
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
    };
    let ran = run(args, &mut reader).await;
    if let Err(error) = &ran {
        reader.error(format!("{error:#}"));
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
