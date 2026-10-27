//! A reader of a room for an agent: asks the room where its broadcast or
//! its meeting is (and in a meeting takes a seat), connects there, and
//! writes what happens in its chat as lines of JSON on stdout and in the
//! files of `<DIR>/events/`; a line of stdin or of `<DIR>/inbox.txt` is
//! said in the chat. `<DIR>/AGENT.md` tells an agent how. It follows a
//! broadcast that ends and begins again.
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

use anyhow::{Context, Result, bail};
use tokio::sync::{mpsc, watch};
use xmoq_again::next_session::{After, NextSession, Pause};
use xmoq_agent::args::{Args, Source};
use xmoq_agent::events::{BroadcastState, Event};
use xmoq_agent::guide::{GUIDE, GUIDE_FILE, call_line};
use xmoq_agent::input::{self, Input};
use xmoq_agent::presence::Presence;
use xmoq_chat_tracks::names::Person;
use xmoq_moqt_io::StreamConfig;
use xmoq_moqt_io::client::{self, CloseWait, Signals};
use xmoq_moqt_io::wt;
use xmoq_room_token::Role;
use xmoq_subscriber::follow::RENDEZVOUS_MS;
use xmoq_viewer::room::{self, NoSeat, Refusal};
use xmoq_viewer_io::room_client::{
    ChatTokens, HeldSeat, ROOM_COMES_WITHIN, RoomClient, relay_address,
};

use crate::attend::{Ended, attend};
use crate::frames::Pictures;

use crate::output::{Output, now_ms};

/// The directory of the pictures in `--out`, with one directory a run in
/// it.
const FRAMES_DIR: &str = "frames";

/// Where the pictures of this run go in `--out`: `frames/<when it began,
/// ms>`, with `/`.
///
/// WHY a directory a run: `Frames` names its files from 1 on each run, and
/// those of a run before in the same `--out` would be written over, and not
/// counted by `--frames-kept`. WHY NOT go on from the numbers there: what
/// names the files is `xmoq-picture`'s. WHY NOT empty the directory: the
/// pictures an agent asked for before are its own.
fn frames_run() -> String {
    format!("{FRAMES_DIR}/{}", now_ms())
}
use crate::stdin::Line;

/// What the reader shares between its parts while it runs.
pub struct Reader {
    out: Output,
    lines: mpsc::Receiver<Line>,
    /// Whether stdin or `inbox.txt` may still bring lines.
    stdin_open: bool,
    stop: watch::Receiver<bool>,
    deadline: Option<tokio::time::Instant>,
    presence: Presence,
    /// Who is in the meeting of `--meeting`, from one session to the next.
    meeting: Option<meeting::KeptMeeting>,
    /// How long the room gets for each of its answers (`--room-wait-ms`).
    room_wait: Duration,
    /// The newest line of the chat written, from one session to the next.
    had: Option<u64>,
    /// The newest piece of speech written, and of which transcript, from one
    /// session to the next.
    speech_had: Option<(Vec<String>, u64)>,
    /// `--frames on`.
    frames: bool,
    /// The picture asked for, from one session to the next.
    frame: FrameAsking,
    /// The id the reader asks a room's meeting for its seat by
    /// (`participant_id`), new where the room says another holds it.
    participant: String,
    /// Whether the session's broadcast was written as begun, and not yet as
    /// ended.
    broadcast_began: bool,
}

/// A random id of 16 hex digits for a seat in a room's meeting, as a page
/// makes its own.
///
/// WHY from the keys `std` makes for each `HashMap` and not a crate of
/// random numbers: the id is no secret (it is a field of a namespace) and
/// has only to differ from the others' in the room, which 64 random bits
/// do, and the keys are random for each process. One is made a run.
fn participant_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let hasher = std::collections::hash_map::RandomState::new().build_hasher();
    format!("{:016x}", hasher.finish())
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
/// or, before any came, for the first key frame, which begins the next
/// Group; and while the reader connects again, for the next session's
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
            Line::InboxFailed(why) => self.error(why),
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
            xmoq_agent::input::MOST_A_LINE
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

    /// Waits `wait_ms` before a next try; `false` where the reader is to end
    /// first.
    async fn pause(&mut self, wait_ms: u64) -> Result<bool> {
        if wait_ms == 0 {
            return Ok(true);
        }
        let wait = Duration::from_millis(wait_ms);
        let waiting = async {
            tokio::time::sleep(wait).await;
            Ok(())
        };
        Ok(self
            .unless_ended("the wait", wait * 2, waiting)
            .await?
            .is_some())
    }
}

/// Milliseconds from `began`, the clock [`NextSession`] is given.
fn ms_since(began: Instant) -> u64 {
    u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// `sub` in `xmoq/transcript/<prefix>`, of the room, as an AUTHORIZATION
/// TOKEN; `Some(None)` where the room gives none, `None` where the reader is
/// to end first.
async fn transcript_token_of(
    client: &RoomClient,
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
            reader.room_wait,
            client.token(Role::Sub, &fields),
        )
        .await?;
    Ok(given.map(|given| given.map(|text| room::authorization_token(&text))))
}

/// Where one session goes: a broadcast, or the relay of a meeting.
pub struct Place {
    url: String,
    cert_sha256: [u8; 32],
    /// The broadcast's namespace; `None` in a meeting.
    namespace: Option<Vec<String>>,
    token: Option<Vec<u8>>,
    chat_tokens: ChatTokens,
    /// The `RENDEZVOUS_TIMEOUT` of the catalog's SUBSCRIBE, where the
    /// broadcast may not have begun.
    catalog_rendezvous_ms: Option<u64>,
    /// The prefix whose transcript is followed; `None` with `--speech off`.
    transcript: Option<Vec<String>>,
    /// `sub` in `xmoq/transcript/<prefix>`, where the room gave one.
    transcript_token: Option<Vec<u8>>,
}

/// The prefix whose transcript is followed (`xmoq_agent::transcript`), with
/// what the room answered and the broadcast's namespace, where there are;
/// none with `--speech off`.
fn transcript_of(
    args: &Args,
    of_the_room: Option<&[String]>,
    broadcast: Option<&[String]>,
) -> Option<Vec<String>> {
    if !args.speech {
        return None;
    }
    let meeting = args.meeting.as_ref().map(|(room, _)| room.as_str());
    xmoq_agent::transcript::prefix(args.transcript.as_deref(), of_the_room, meeting, broadcast)
}

/// Where the broadcast or the meeting is, with the tokens of the room.
/// `None` when the reader is to end first.
///
/// # Errors
///
/// What [`after_room_failure`] ends the reader with; what else fails in a
/// room has it asked again ([`place_in_room`]).
async fn place(
    args: &Args,
    client: Option<&RoomClient>,
    seat: &mut Option<HeldSeat>,
    reader: &mut Reader,
) -> Result<Option<Place>> {
    match (&args.source, client) {
        (
            Source::Relay {
                url,
                cert_sha256,
                namespace,
            },
            _,
        ) => Ok(Some(at_relay(
            args,
            url,
            *cert_sha256,
            namespace.as_deref(),
        ))),
        (Source::Room { ask_token, .. }, Some(client)) => {
            place_in_room(args, client, *ask_token, seat, reader).await
        }
        (Source::Room { .. }, None) => bail!("a room without its client"),
    }
}

/// [`place`] in the room of `client`, asked again where it fails, by the
/// rule of the next session (`Pause::VIEWER` from the try before), with
/// an `error` where the failure is not the one told last.
///
/// WHY on and on, where `xmoq-watch` ends: as for a first session that
/// fails (`run`), nobody looks at the reader to start it again, and the room
/// may be back (a Worker restarted, a room that answers late or answers
/// what it should again). WHY not at once: a failure other than a refusal
/// (a 404, an answer the reader does not take) comes back as fast as it is
/// asked. WHY the same failure told once: an agent reads `events/` to
/// its end, and a room that is wrong for hours would fill it with one line.
async fn place_in_room(
    args: &Args,
    client: &RoomClient,
    ask_token: bool,
    seat: &mut Option<HeldSeat>,
    reader: &mut Reader,
) -> Result<Option<Place>> {
    let mut rule = NextSession::new(Pause::VIEWER, true, false);
    let began = Instant::now();
    let mut told: Option<String> = None;
    let at = client.address().base.clone();
    loop {
        if !reader
            .pause(rule.next(&at, ms_since(began)).wait_ms)
            .await?
        {
            return Ok(None);
        }
        let error = match place_in(args, client, ask_token, seat, reader).await {
            Ok(placed) => return Ok(placed),
            Err(error) => error,
        };
        match after_room_failure(error, &mut told) {
            AfterRoom::End(error) => return Err(error),
            AfterRoom::Again { tell, new_id } => {
                if new_id {
                    reader.participant = participant_id();
                }
                if let Some(message) = tell {
                    reader.error(message);
                }
                rule.ended(Ended::Failed { began: false });
            }
        }
    }
}

/// The room's answer that is no seat, as an error.
#[derive(Debug)]
struct NoSeatGiven(NoSeat);

impl std::fmt::Display for NoSeatGiven {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            NoSeat::Full { most } => write!(formatter, "the meeting is full: it takes {most}"),
            NoSeat::Refused(Refusal::Taken) => {
                formatter.write_str("the room gave no seat: the reader's id is another's")
            }
            NoSeat::Refused(Refusal::NotAParticipant) => {
                formatter.write_str("the room gave no seat: the reader's id is not a participant's")
            }
            NoSeat::Refused(Refusal::NotAMeetingRoom) => {
                formatter.write_str("the room gave no seat: the name is not a meeting room's")
            }
            NoSeat::Gone => formatter.write_str("the room gave no seat: it is gone"),
        }
    }
}

impl std::error::Error for NoSeatGiven {}

/// What a failure of the room comes to in [`place_in_room`].
#[derive(Debug)]
enum AfterRoom {
    /// The reader ends with it.
    End(anyhow::Error),
    /// The room is asked again, with the line to tell where it is not the
    /// one told last, and with a new id of the reader where `new_id`.
    Again { tell: Option<String>, new_id: bool },
}

/// What a failure of the room comes to: the reader ends with a full meeting
/// and with a seat refused for the flags (the meeting's name, or an id that
/// is no participant's), which no try again changes; the room is asked
/// again after any other.
///
/// WHY a full meeting ends the reader: whether an agent waits for a seat
/// that people of the meeting would take is nothing this decides.
/// FIXME: decide with the owner whether an agent waits for a seat of a full
/// meeting, and how long.
///
/// WHY a new id where the room says another holds it: the id is 64 random
/// bits, so it is another's by chance alone, or the reader's own seat of a
/// session before that it lost and that has not run out yet, which it
/// cannot take again without its lease; with the same id the room would
/// refuse it until then, and for ever in the first case.
fn after_room_failure(error: anyhow::Error, told: &mut Option<String>) -> AfterRoom {
    let no_seat = error.downcast_ref::<NoSeatGiven>().map(|given| given.0);
    let new_id = match no_seat {
        Some(
            NoSeat::Full { .. }
            | NoSeat::Refused(Refusal::NotAParticipant | Refusal::NotAMeetingRoom),
        ) => return AfterRoom::End(error),
        Some(NoSeat::Refused(Refusal::Taken)) => true,
        Some(NoSeat::Gone) | None => false,
    };
    let message = format!("the room: {error:#}; asking it again");
    if told.as_ref() == Some(&message) {
        return AfterRoom::Again { tell: None, new_id };
    }
    *told = Some(message.clone());
    AfterRoom::Again {
        tell: Some(message),
        new_id,
    }
}

/// [`place`] in the room of `client`, asked once: the room, and in a
/// meeting the reader's seat, taken where it holds none.
async fn place_in(
    args: &Args,
    client: &RoomClient,
    ask_token: bool,
    seat: &mut Option<HeldSeat>,
    reader: &mut Reader,
) -> Result<Option<Place>> {
    let asking = client.room_when_there(ROOM_COMES_WITHIN);
    // The last try may begin as the refusals' time ends.
    let wait = ROOM_COMES_WITHIN + reader.room_wait;
    let Some(answered) = reader.unless_ended("the room", wait, asking).await? else {
        return Ok(None);
    };
    let (relay, transcript, namespace, catalog_rendezvous_ms) = match answered {
        room::Room::Broadcast(broadcast) => {
            let transcript = transcript_of(
                args,
                broadcast.transcript.as_deref(),
                Some(&broadcast.namespace),
            );
            // WHY the rendezvous: the room names its broadcast whether it
            // has begun or not, and says nothing when it begins.
            (
                broadcast.relay,
                transcript,
                Some(broadcast.namespace),
                Some(RENDEZVOUS_MS),
            )
        }
        room::Room::Meeting(meeting) => {
            let expected = xmoq_meeting_tracks::prefix(&client.address().name);
            if meeting.namespace != expected {
                bail!(
                    "the room's meeting is under {}, not {}",
                    meeting.namespace.join("/"),
                    expected.join("/")
                );
            }
            (
                meeting.relay,
                transcript_of(args, meeting.transcript.as_deref(), None),
                None,
                None,
            )
        }
    };
    let (url, cert_sha256) = relay_address(relay)?;
    let mut token = None;
    let mut chat_tokens = ChatTokens::default();
    let mut transcript_token = None;
    if ask_token {
        let given = match &namespace {
            Some(namespace) => {
                reader
                    .unless_ended(
                        "the room's token",
                        reader.room_wait,
                        client.token(Role::Sub, namespace),
                    )
                    .await?
            }
            None => seat_token_of(client, seat, reader).await?,
        };
        let Some(given) = given else {
            return Ok(None);
        };
        token = given.map(|text| room::authorization_token(&text));
        let Some(tokens) = reader
            .unless_ended(
                "the room's tokens for the chat",
                reader.room_wait,
                client.chat_tokens(&person_of(args)),
            )
            .await?
        else {
            return Ok(None);
        };
        chat_tokens = tokens;
        if let Some(prefix) = &transcript {
            let Some(given) = transcript_token_of(client, reader, prefix).await? else {
                return Ok(None);
            };
            transcript_token = given;
        }
    }
    Ok(Some(Place {
        url,
        cert_sha256,
        namespace,
        token,
        chat_tokens,
        catalog_rendezvous_ms,
        transcript,
        transcript_token,
    }))
}

/// `sub` in `xmoq/meet/<room>/*` of the reader's seat in the room's
/// meeting, as its text: of the seat held, or of one taken now; `Some(None)`
/// where the room gives no tokens, `None` where the reader is to end first.
///
/// WHY a seat, though the reader announces nothing in the meeting: the room
/// counts its seats as it gives them, and a reader takes one of them as a
/// participant's page does. Whether those present learn that an agent takes
/// part is undecided (`docs/open-questions.md`); a seat says nothing to
/// them. WHY the seat's newest token on every session: it is renewed with a
/// new one, and a token is good for a time from when the room made it.
///
/// # Errors
///
/// [`NoSeatGiven`], a seat for another namespace than the meeting's, and
/// what fails in asking the room.
async fn seat_token_of(
    client: &RoomClient,
    seat: &mut Option<HeldSeat>,
    reader: &mut Reader,
) -> Result<Option<Option<String>>> {
    if seat.as_ref().is_some_and(HeldSeat::is_lost) {
        *seat = None;
    }
    if seat.is_none() {
        let participant = reader.participant.clone();
        let Some(answered) = reader
            .unless_ended(
                "the room's seat in its meeting",
                reader.room_wait,
                client.seat(&participant),
            )
            .await?
        else {
            return Ok(None);
        };
        let held = answered.map_err(NoSeatGiven)?;
        let expected = xmoq_meeting_tracks::seat_subscribe(&client.address().name);
        let subscribe = held.seat().borrow().subscribe.clone();
        if subscribe != expected {
            bail!(
                "the room's seat is for {}, not {}",
                subscribe.join("/"),
                expected.join("/")
            );
        }
        *seat = Some(held);
    }
    Ok(Some(seat.as_ref().and_then(|held| {
        held.seat().borrow().subscribe_token.clone()
    })))
}

/// Writes that the broadcast of `place` began (its catalog came) or ended;
/// nothing in a meeting.
pub(crate) fn tell_broadcast(reader: &mut Reader, place: &Place, state: BroadcastState) {
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

async fn run(
    args: &Args,
    reader: &mut Reader,
    pictures: &mut Option<Pictures>,
    seat: &mut Option<HeldSeat>,
) -> Result<()> {
    let client = match &args.source {
        Source::Room { address, .. } => Some(RoomClient::new(address.clone(), reader.room_wait)?),
        Source::Relay { .. } => None,
    };
    // WHY a first failure does not end the reader where it follows a room
    // (where `xmoq-watch` ends): an agent starts it, often as the relay and
    // the room start (`scripts/agent-demo.sh`), and nobody looks at it to
    // start it again; it waits as for a broadcast that broke off.
    let follows = matches!(args.source, Source::Room { .. });
    let mut rule = NextSession::new(Pause::VIEWER, follows, false);
    let began = Instant::now();
    loop {
        let Some(mut place) = place(args, client.as_ref(), seat, reader).await? else {
            return Ok(());
        };
        let next = rule.next(&place.url, ms_since(began));
        place.url = next.url;
        if !reader.pause(next.wait_ms).await? {
            return Ok(());
        }
        let streams = StreamConfig::default();
        let connecting = async { Ok(wt::connect(&place.url, place.cert_sha256, &streams).await?) };
        let connect_wait = Duration::from_millis(args.connect_wait_ms);
        let mut connected = false;
        let attended = match reader
            .unless_ended("the relay", connect_wait, connecting)
            .await
        {
            Ok(None) => return Ok(()),
            Ok(Some(connection)) => {
                connected = true;
                let connection = Arc::new(connection);
                // The peer's end of a session that failed is not written:
                // the error of the session is (below).
                let (attended, _) = client::on_connection(
                    &connection,
                    CloseWait::VIEWER,
                    async |connection, kept| {
                        // Boxed: the session's future is the largest by
                        // far, and the reader's own would grow by it.
                        Box::pin(attend(
                            args, connection, &place, seat, reader, pictures, kept,
                        ))
                        .await
                    },
                )
                .await;
                attended
            }
            Err(error) => Err(error),
        };
        reader.presence.session_ended();
        if let Some(kept) = reader.meeting.as_mut() {
            kept.session_ended();
        }
        // Paired with the `began` the session wrote, on every way out of it.
        if std::mem::take(&mut reader.broadcast_began) {
            tell_broadcast(reader, &place, BroadcastState::Ended);
        }
        // WHY `began` is that the relay took the connection: a relay that
        // turns the reader down at its first try is to be seen, as
        // `xmoq-watch` has it for one that plays nothing.
        let end = match &attended {
            Ok(end) => end.clone(),
            Err(_) => Ended::Failed { began: connected },
        };
        match (rule.ended(end), attended) {
            (After::Connect, Err(error)) => reader.error(format!("the session ended: {error:#}")),
            (After::Connect, Ok(_)) => {}
            (After::End, _) => return Ok(()),
            (After::EndWithError, attended) => return attended.map(|_| ()),
        }
    }
}

/// Where a reader without a room connects: the relay of `--url`.
fn at_relay(args: &Args, url: &str, cert_sha256: [u8; 32], namespace: Option<&[String]>) -> Place {
    // A meeting has a broadcast for each participant, and the transcript of
    // all of them under its prefix (`transcript_of`).
    let namespace = namespace.map(<[String]>::to_vec);
    Place {
        url: url.to_string(),
        cert_sha256,
        transcript: transcript_of(args, None, namespace.as_deref()),
        namespace,
        token: None,
        chat_tokens: ChatTokens::default(),
        catalog_rendezvous_ms: None,
        transcript_token: None,
    }
}

/// Puts the guide for an agent in `out`, and says on stderr the line a
/// person gives an agent to have it take part, with the folder in full.
///
/// WHY written anew each run: the guide is of this version of the reader,
/// and one left by a run before may tell of another.
fn put_guide(out: &Path) -> Result<()> {
    let path = out.join(GUIDE_FILE);
    std::fs::write(&path, GUIDE).with_context(|| format!("{}", path.display()))?;
    let folder = std::fs::canonicalize(out).unwrap_or_else(|_| out.to_path_buf());
    eprintln!("{}", call_line(&folder.display().to_string()));
    Ok(())
}

async fn read(args: &Args) -> Result<()> {
    let stop = client::on_signals(Signals::VIEWER);
    let out = Output::open(Path::new(&args.out))?;
    put_guide(Path::new(&args.out))?;
    let mut reader = Reader {
        out,
        lines: stdin::lines(Path::new(&args.out).join(xmoq_agent::inbox::INBOX_FILE)),
        stdin_open: true,
        stop,
        deadline: args
            .seconds
            .map(|seconds| tokio::time::Instant::now() + Duration::from_secs(seconds)),
        presence: Presence::new(&args.room, &args.name),
        meeting: meeting::KeptMeeting::of(args),
        room_wait: Duration::from_millis(args.room_wait_ms),
        had: None,
        speech_had: None,
        frames: args.frames,
        frame: FrameAsking::default(),
        participant: participant_id(),
        broadcast_began: false,
    };
    let mut pictures = if args.frames {
        Some(Pictures::start(
            args.pictures,
            Path::new(&args.out),
            frames_run(),
        )?)
    } else {
        None
    };
    let mut seat = None;
    let ran = run(args, &mut reader, &mut pictures, &mut seat).await;
    if let Err(error) = &ran {
        reader.error(format!("{error:#}"));
    }
    // On every way out of `run`, a signal's as well: the seat is free at
    // once, and not only when it runs out.
    if let Some(seat) = seat
        && let Err(error) = seat.leave().await
    {
        reader.error(format!("leaving the seat in the meeting: {error:#}"));
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

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use xmoq_viewer::room::{NoSeat, Refusal};

    use super::{AfterRoom, NoSeatGiven, after_room_failure};

    /// The line told and whether the reader takes a new id, or the error
    /// the reader ends with.
    type Told = Result<(Option<&'static str>, bool), &'static str>;

    #[test]
    fn a_room_that_fails_is_asked_again_and_told_once_for_the_same_failure_until_the_flags_are_wrong()
     {
        // (the failure of the room, in the order they come; the line told
        //  and whether the reader takes a new id, or the error the reader
        //  ends with)
        let failures: [(anyhow::Error, Told); 9] = [
            (
                anyhow!("asking the room http://h/meetings/m"),
                Ok((
                    Some("the room: asking the room http://h/meetings/m; asking it again"),
                    false,
                )),
            ),
            (
                anyhow!("asking the room http://h/meetings/m"),
                Ok((None, false)),
            ),
            (
                anyhow!("the room's token: nothing within 5000 ms"),
                Ok((
                    Some("the room: the room's token: nothing within 5000 ms; asking it again"),
                    false,
                )),
            ),
            (
                NoSeatGiven(NoSeat::Refused(Refusal::Taken)).into(),
                Ok((
                    Some(
                        "the room: the room gave no seat: the reader's id is another's; asking it again",
                    ),
                    true,
                )),
            ),
            // Told once, and a new id each time.
            (
                NoSeatGiven(NoSeat::Refused(Refusal::Taken)).into(),
                Ok((None, true)),
            ),
            (
                anyhow!("asking the room http://h/meetings/m"),
                Ok((
                    Some("the room: asking the room http://h/meetings/m; asking it again"),
                    false,
                )),
            ),
            (
                NoSeatGiven(NoSeat::Refused(Refusal::NotAParticipant)).into(),
                Err("the room gave no seat: the reader's id is not a participant's"),
            ),
            (
                NoSeatGiven(NoSeat::Refused(Refusal::NotAMeetingRoom)).into(),
                Err("the room gave no seat: the name is not a meeting room's"),
            ),
            (
                NoSeatGiven(NoSeat::Full { most: 49 }).into(),
                Err("the meeting is full: it takes 49"),
            ),
        ];
        let mut told = None;
        for (failure, expected) in failures {
            let came = match after_room_failure(failure, &mut told) {
                AfterRoom::End(error) => Err(error.to_string()),
                AfterRoom::Again { tell, new_id } => Ok((tell, new_id)),
            };
            assert_eq!(
                came.as_ref()
                    .map(|(tell, new_id)| (tell.as_deref(), *new_id))
                    .map_err(String::as_str),
                expected
            );
        }
    }
}
