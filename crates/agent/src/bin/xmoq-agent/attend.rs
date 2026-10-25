//! One session at a broadcast: the chat on tracks, who is in it, the lines
//! of stdin said in it, and the pictures of the video asked for.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc;
use xmoq_again::AskAgain;
use xmoq_agent::events::{Asked as FrameAsked, Event};
use xmoq_agent::input::{self, Input};
use xmoq_agent::presence::SETTLE_MS;
use xmoq_chat_tracks::names::{person_of as person_of_namespace, room_namespace};
use xmoq_chat_tracks::payloads::Line as ChatLine;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::{ObjectStatus, SubgroupObject};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::Parameter;
use xmoq_moqt::session::{
    ControlEvent, PeerRequestId, RequestEvent, RequestId, Role, Session, Transport,
};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, RequestReader, RequestStreamEvent, SendStream, StreamConfig, StreamStops, Trace,
    UniEvent, read_request_streams, read_uni_streams, send_message,
};
use xmoq_transcript_tracks::names::{SPEECH, transcript_namespace};
use xmoq_transcript_tracks::payloads::{Speech, read_speech};
use xmoq_viewer_io::chat_tracks::{ChatTracks, Out};
use xmoq_viewer_io::follow::{Asking, Follower, Out as FollowOut, Track};
use xmoq_viewer_io::room_socket::RoomSocket;
use xmoq_viewer_io::session::{self, Events, GoneAway, Requests, new_url, timestamp_of};

use crate::frames::{Job, Pictures};
use crate::meeting::InMeeting;
use crate::stdin::Line;
use crate::video::{Asking as VideoAsking, Video};
use crate::{Outside, Place, Reader, person_of};

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

/// How a session ended.
pub enum Ended {
    /// The reader is to end.
    Quit,
    /// The broadcast ended or moved: the room says what comes next.
    Again,
    /// The relay sent GOAWAY (§3.6): the reader connects again, to the URL it
    /// named, or to the same one where it named none.
    Moved(Option<String>),
}

/// What has to stay until the session is closed, on every way out of
/// [`attend`], so its caller keeps it. WHY, as in `xmoq-watch`: a relay that
/// sees the control stream end, or its own stopped, closes the session
/// first, with `PROTOCOL_VIOLATION` (§3.3).
#[derive(Default)]
pub struct UntilClosed {
    control: Option<Send>,
    readers: Option<xmoq_moqt_io::IncomingStreams>,
    requests: Option<xmoq_moqt_io::IncomingStreams>,
}

/// The `SUBSCRIBE_NAMESPACE` for `xmoq/chat/<room>`, which tells who is in
/// the chat.
pub(crate) struct Watching {
    pub(crate) request: RequestId,
    pub(crate) reader: RequestReader<Recv>,
    /// WHY kept and not finished: a FIN of the subscriber ends a
    /// `SUBSCRIBE_NAMESPACE` (§6.1).
    send: Send,
}

impl Watching {
    pub(crate) fn cancel(self) {
        self.send.reset(stream_reset_code::CANCELLED);
        self.reader.stop(stream_reset_code::CANCELLED);
    }
}

/// Asks for the namespaces of the room's chat. A failure is the caller's to
/// tell, and to ask again after.
pub(crate) async fn watch(
    connection: &WtConnection,
    session: &mut Session,
    room: &TrackNamespace,
    token: Option<&[u8]>,
    open_wait_ms: u64,
) -> Result<Watching> {
    let parameters = token
        .map(|token| Parameter::AuthorizationToken(token.to_vec()))
        .into_iter()
        .collect();
    let (request, message) = session.subscribe_namespace(room.clone(), parameters)?;
    let opened =
        tokio::time::timeout(Duration::from_millis(open_wait_ms), connection.open_bi()).await;
    let Ok(opened) = opened else {
        session.cancel(request);
        bail!("the relay allowed no stream for SUBSCRIBE_NAMESPACE within {open_wait_ms} ms");
    };
    let (mut send, recv) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            session.cancel(request);
            return Err(error.into());
        }
    };
    let trace = Trace::default();
    if let Err(error) = send_message(&mut send, &message, &trace, "subscribe-namespace").await {
        session.cancel(request);
        return Err(error.into());
    }
    Ok(Watching {
        request,
        reader: RequestReader::new(recv, &trace, "subscribe-namespace"),
        send,
    })
}

/// Waits for `item` where there is one, and for ever where there is none.
pub(crate) async fn on<T>(item: Option<impl std::future::Future<Output = T>>) -> T {
    match item {
        Some(future) => future.await,
        None => std::future::pending().await,
    }
}

/// The event of a line of the chat.
fn chat_event(line: ChatLine) -> Event {
    Event::Chat {
        number: line.number,
        from: line.name,
        text: line.text,
        sent_at: line.sent_at,
        gathered_at: line.at,
    }
}

/// The chat on tracks of the session, and who is in it.
struct Chat {
    tracks: ChatTracks,
    room: TrackNamespace,
    /// The `sub` token of the room's namespace, where the room gave one.
    token: Option<Vec<u8>>,
    open_wait_ms: u64,
    watching: Option<Watching>,
    /// When to ask for the namespaces again, after one that failed or ended.
    watch_at: Option<tokio::time::Instant>,
    again: AskAgain,
    /// When those not told of again on this session are taken to have left.
    settle_at: Option<tokio::time::Instant>,
}

impl Chat {
    /// Asks for the namespaces of the room, and tells what failed.
    async fn watch(
        &mut self,
        session: &mut Session,
        connection: &WtConnection,
        reader: &mut Reader,
    ) {
        self.watch_at = None;
        match watch(
            connection,
            session,
            &self.room,
            self.token.as_deref(),
            self.open_wait_ms,
        )
        .await
        {
            Ok(watching) => self.watching = Some(watching),
            Err(error) => self.watch_failed(reader, &format!("{error:#}")),
        }
    }

    fn watch_failed(&mut self, reader: &mut Reader, how: &str) {
        if let Some(watching) = self.watching.take() {
            watching.cancel();
        }
        let pause = self.again.after_end();
        self.watch_at = Some(tokio::time::Instant::now() + Duration::from_millis(pause));
        reader.error(format!(
            "who is in the chat: {how}; asking again in {pause} ms"
        ));
    }

    /// What came on the stream of the `SUBSCRIBE_NAMESPACE`.
    fn on_watching(
        &mut self,
        session: &mut Session,
        reader: &mut Reader,
        event: Result<RequestStreamEvent, xmoq_moqt_io::RequestReadError>,
    ) -> Option<Ended> {
        let request = self.watching.as_ref().map(|watching| watching.request)?;
        let message = match event {
            Ok(RequestStreamEvent::Message(message)) => message,
            Ok(RequestStreamEvent::Closed(closed)) => {
                session.cancel(request);
                self.watch_failed(reader, &format!("the relay closed it ({closed:?})"));
                return None;
            }
            // §10.18: a reset is as though each namespace got NAMESPACE_DONE;
            // those not told of again are taken to have left when the next
            // one settles.
            Err(error) => {
                session.cancel(request);
                reader.presence.session_ended();
                self.watch_failed(reader, &format!("{error}"));
                return None;
            }
        };
        match session.on_request_message(request, message) {
            Ok(RequestEvent::NamespacesSubscribed) => {
                self.again.subscribed();
                self.settle_at =
                    Some(tokio::time::Instant::now() + Duration::from_millis(SETTLE_MS));
            }
            Ok(RequestEvent::Namespace(namespace)) => {
                if let Some(event) = person_of_namespace(&namespace)
                    .and_then(|person| reader.presence.named(&person))
                {
                    reader.tell(&event);
                }
            }
            Ok(RequestEvent::NamespaceDone(namespace)) => {
                if let Some(event) =
                    person_of_namespace(&namespace).and_then(|person| reader.presence.done(&person))
                {
                    reader.tell(&event);
                }
            }
            Ok(RequestEvent::Rejected(error)) => {
                session.cancel(request);
                self.watch_failed(reader, &format!("refused with {:#x}", error.error_code));
            }
            Ok(RequestEvent::GoAway(go_away)) => return Some(Ended::Moved(new_url(&go_away))),
            Ok(other) => reader.error(format!("who is in the chat: {other:?}")),
            // More namespaces than the session keeps end that request alone.
            Err(error) => {
                session.cancel(request);
                self.watch_failed(reader, &format!("{error}"));
            }
        }
        None
    }
}

/// What `/frame` gets once the thread of the pictures is gone (it panicked).
const THREAD_ENDED: &str = "frame: the thread of the pictures ended";

/// Why there are no pictures on this session.
enum NoVideo {
    /// `--frames off`.
    Off,
    /// The session is at the room's relay for the chat, and the room has no
    /// broadcast yet.
    NoBroadcast,
    /// The catalog or the subscription failed, or the video was given up.
    Failed(String),
}

impl NoVideo {
    fn message(&self) -> String {
        match self {
            Self::Off => "frame: --frames off".to_string(),
            Self::NoBroadcast => "frame: no broadcast yet".to_string(),
            Self::Failed(why) => format!("frame: no video: {why}"),
        }
    }
}

/// Tells `Frames` what was asked for, or given up, while nothing could tell
/// it: `/frame` before this session's video, or a wait that ran out between
/// sessions. A `/frame` that this session has no video for is told as such.
async fn settle_frame_asking(
    reader: &mut Reader,
    pictures: Option<&mut Pictures>,
    video: &Result<Video, NoVideo>,
) {
    if !reader.frame.forgo_unsent && !reader.frame.unsent {
        return;
    }
    let Some(pictures) = pictures else {
        if std::mem::take(&mut reader.frame.unsent) {
            reader.frame.until = None;
            reader.error(THREAD_ENDED.to_string());
        }
        reader.frame.forgo_unsent = false;
        return;
    };
    if std::mem::take(&mut reader.frame.forgo_unsent) {
        pictures.tell(Job::Forgo).await;
    }
    if std::mem::take(&mut reader.frame.unsent) {
        match video {
            Ok(_) => pictures.tell(Job::Ask).await,
            Err(why) => {
                reader.frame.until = None;
                reader.error(why.message());
            }
        }
    }
}

/// An Object of the video, for the thread of its pictures.
fn on_video_object(
    pictures: &mut Pictures,
    reader: &mut Reader,
    group: u64,
    object: &SubgroupObject,
) {
    if object.status != ObjectStatus::Normal {
        return;
    }
    // An Object without a Timestamp of LOC, or that is no LOC, has no
    // picture for an agent; `Frames` would not know where it is.
    let Some(timestamp_us) = timestamp_of(object) else {
        return;
    };
    let Ok(annex_b) = loc::to_annex_b(&object.payload) else {
        return;
    };
    let given = pictures.object(Job::Object {
        group,
        object: object.object_id,
        timestamp_us,
        annex_b,
    });
    if !given && !pictures.leaving_out {
        reader.error(format!(
            "frame: the pictures are behind; {} Objects of the video left out so far",
            pictures.left_out
        ));
    }
    pictures.leaving_out = !given;
}

/// How long the relay is asked to hold the SUBSCRIBE to `speech` for a
/// transcriber to publish it: the most the own relay holds, as for the
/// chat's `lines`. WHY held: a transcriber that starts after the reader has
/// its pieces read from the start.
const SPEECH_RENDEZVOUS_MS: u64 = 30_000;

/// The most an Object of `speech` may be. **Provisional**: a piece of a few
/// seconds is some hundred bytes of text.
const SPEECH_PAYLOAD_MOST: usize = 64 * 1024;

/// The most bytes of the stream of the FETCH of `speech`: the pieces
/// fetched, with room. **Provisional.**
const SPEECH_FETCH_MOST: usize = 1024 * 1024;

/// How many of the pieces missed while the subscription was away are
/// fetched: as many as of the chat's lines by default. **Provisional.**
const SPEECH_CATCH_UP: u64 = xmoq_viewer::chat_track::CATCH_UP_LINES;

/// Writes what came of the transcript.
fn on_speech(reader: &mut Reader, out: FollowOut<Speech>) {
    match out {
        FollowOut::Item(piece) => reader.tell(&Event::Speech {
            number: piece.number,
            from: piece.from,
            text: piece.text,
            start_us: piece.start_us,
            end_us: piece.end_us,
            read_at: piece.at,
        }),
        FollowOut::Gap(gap) => reader.error(format!(
            "speech: {} pieces not fetched ({} to {})",
            gap.before - gap.after - 1,
            gap.after + 1,
            gap.before - 1
        )),
        FollowOut::Note(note) => eprintln!("{note}"),
    }
}

/// Keeps the newest piece written, for the next session.
fn remember_speech(reader: &mut Reader, place: &Place, follower: &Follower<Speech>) {
    if let (Some(prefix), Some(last)) = (&place.transcript, follower.last_shown()) {
        reader.speech_had = Some((prefix.clone(), last));
    }
}

/// What a line of stdin asks for, where the session has to do it.
enum Wants {
    Nothing,
    /// `/frame`, and in a meeting whose picture.
    Frame(Option<String>),
}

/// Says a line of stdin in the chat, or tells what it asks for.
fn on_line(
    chat: &mut ChatTracks,
    connection: &Arc<WtConnection>,
    reader: &mut Reader,
    line: Line,
) -> Wants {
    let text = match line {
        Line::TooLong(length) => {
            reader.too_long(length);
            return Wants::Nothing;
        }
        Line::Read(text) => text,
    };
    match input::read(&text) {
        Input::Nothing => {}
        Input::Say(text) => {
            let length = text.len();
            if !chat.say(connection, text) {
                reader.error(format!(
                    "said nothing: a line of {length} bytes is more than {} bytes, or {} lines wait for the gatherer already",
                    xmoq_chat_tracks::limits::TEXT_BYTES,
                    xmoq_viewer::chat_track::SAID_WAITING
                ));
            }
        }
        Input::Frame(of) => return Wants::Frame(of),
        Input::Unknown(name) => reader.unknown_command(&name),
    }
    Wants::Nothing
}

/// Reads one broadcast's session until it ends, moves, or the reader is
/// ended.
#[expect(
    clippy::too_many_lines,
    reason = "one select over everything that can happen on the session: cut in two, each half \
              would need the other's state"
)]
pub async fn attend(
    args: &xmoq_agent::args::Args,
    connection: &Arc<WtConnection>,
    place: &Place,
    room: &mut Option<RoomSocket>,
    reader: &mut Reader,
    pictures: &mut Option<Pictures>,
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
    let (asks, mut asked) = mpsc::channel(16);
    kept.requests = Some(read_request_streams(
        Arc::clone(connection),
        StreamConfig::default(),
        stops.clone(),
        asks,
        |event| event,
    ));
    let mut refused: HashMap<u64, PeerRequestId> = HashMap::new();
    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    let setup = session::setup(
        connection,
        &mut session,
        &mut events,
        &mut kept.control,
        place.token.as_deref(),
        implementation,
        args.setup_wait_ms,
    );
    if reader
        .unless_ended(
            "the relay's SETUP",
            Duration::from_millis(args.setup_wait_ms) * 2,
            setup,
        )
        .await?
        .is_none()
    {
        return Ok(Ended::Quit);
    }
    // WHY before the chat: reading the catalog takes every event of the
    // session meanwhile (`Requests::catalog`), and the chat's lines and its
    // FETCH would be taken with them.
    // Where there is a broadcast to subscribe to, this is replaced below.
    let mut video = Err(if pictures.is_some() {
        NoVideo::NoBroadcast
    } else {
        NoVideo::Off
    });
    if let (Some(pictures), Some(namespace)) = (pictures.as_mut(), &place.namespace) {
        let broadcast = TrackNamespace::new(
            namespace
                .iter()
                .map(|field| field.as_bytes().to_vec())
                .collect(),
        )?;
        let asking_with = VideoAsking {
            connection: Arc::clone(connection),
            namespace: broadcast,
            token: place.token.clone(),
            answer_wait_ms: args.answer_wait_ms,
            open_wait_ms: args.open_wait_ms,
        };
        let subscribed = Video::begin(asking_with, &mut session, &mut events, &stops);
        // The catalog's SUBSCRIBE and FETCH, the catalog itself, and the
        // video's SUBSCRIBE.
        let wait = Duration::from_millis(args.answer_wait_ms * 3) + Duration::from_secs(5);
        video = match reader.unless_ended("the video", wait, subscribed).await {
            Ok(None) => return Ok(Ended::Quit),
            Ok(Some(video)) => {
                // Before the first Object of this session's video reaches
                // the thread: the Objects go through the loop below.
                pictures.tell(Job::BeginAgain).await;
                Ok(video)
            }
            Err(error) => {
                if let Some(GoneAway(go_away)) =
                    error.chain().find_map(|cause| cause.downcast_ref())
                {
                    return Ok(Ended::Moved(new_url(go_away)));
                }
                if error
                    .chain()
                    .any(<dyn std::error::Error>::is::<xmoq_moqt::Error>)
                {
                    return Err(error);
                }
                let why = NoVideo::Failed(format!("{error:#}"));
                reader.error(why.message());
                Err(why)
            }
        };
    }
    let room_namespace = room_namespace(&args.room)?;
    let person = person_of(args);
    let begun = ChatTracks::begin(
        Requests {
            connection,
            session: &mut session,
            namespace: &room_namespace,
            token: place.chat_tokens.subscribe.as_deref(),
            answer_wait_ms: args.answer_wait_ms,
            open_wait_ms: args.open_wait_ms,
            rendezvous_ms: None,
        },
        place.chat_tokens.publish.as_deref(),
        connection,
        &stops,
        &person,
        args.catch_up,
        reader.had,
    );
    let wait = Duration::from_millis(args.open_wait_ms + args.answer_wait_ms);
    let Some(tracks) = reader
        .unless_ended("the chat", wait, events.while_waiting(begun))
        .await?
        .transpose()?
    else {
        return Ok(Ended::Quit);
    };
    let mut chat = Chat {
        tracks,
        room: room_namespace,
        token: place.chat_tokens.subscribe.clone(),
        open_wait_ms: args.open_wait_ms,
        watching: None,
        watch_at: None,
        again: AskAgain::default(),
        settle_at: None,
    };
    chat.watch(&mut session, connection, reader).await;
    eprintln!("XMOQ_AGENT_CHAT room={} name={}", args.room, args.name);
    let mut meeting =
        InMeeting::begin(args, connection, place.token.clone(), &mut session, reader).await?;
    let mut speech = match &place.transcript {
        Some(prefix) => {
            let listened = TrackNamespace::new(
                prefix
                    .iter()
                    .map(|field| field.as_bytes().to_vec())
                    .collect(),
            )?;
            let had = reader
                .speech_had
                .as_ref()
                .filter(|(of, _)| of == prefix)
                .map(|(_, had)| *had);
            Some(Follower::new(
                Track {
                    namespace: transcript_namespace(&listened)?,
                    name: SPEECH,
                    rendezvous_ms: Some(SPEECH_RENDEZVOUS_MS),
                    payload_most: SPEECH_PAYLOAD_MOST,
                    fetch_most: SPEECH_FETCH_MOST,
                    catch_up: SPEECH_CATCH_UP,
                    note: "XMOQ_AGENT_SPEECH",
                    fetch_note: "XMOQ_AGENT_SPEECH",
                    read: |number, payload| read_speech(number, payload).ok(),
                },
                Asking {
                    connection: Arc::clone(connection),
                    stops: stops.clone(),
                    token: place.transcript_token.clone(),
                    answer_wait_ms: args.answer_wait_ms,
                    open_wait_ms: args.open_wait_ms,
                },
                had,
            ))
        }
        None => None,
    };

    let ended = loop {
        let Chat {
            tracks,
            watching,
            watch_at,
            settle_at,
            ..
        } = &mut chat;
        // What was asked for or given up while the loop could not tell
        // `Frames`: before this session's video, or in the wait for the room.
        // In a meeting, a picture asked for then is asked for now, of whom
        // it named.
        if let (Some(in_meeting), Some(taking)) = (meeting.as_mut(), pictures.as_mut())
            && std::mem::take(&mut reader.frame.unsent)
        {
            let of = reader.frame.of.take();
            if let Some(ended) = in_meeting
                .frame(
                    of.as_deref(),
                    &mut session,
                    &mut events,
                    &stops,
                    taking,
                    reader,
                )
                .await?
            {
                break ended;
            }
        }
        settle_frame_asking(reader, pictures.as_mut(), &video).await;
        let told = pictures.as_mut().map(|pictures| &mut pictures.told);
        tokio::select! {
            outside = reader.outside() => match outside {
                Outside::End => break Ended::Quit,
                Outside::Line(line) => match on_line(&mut chat.tracks, connection, reader, line) {
                    Wants::Nothing => {}
                    Wants::Frame(of) if meeting.is_some() => {
                        if let (Some(in_meeting), Some(taking)) = (meeting.as_mut(), pictures.as_mut()) {
                            if let Some(ended) = in_meeting
                                .frame(of.as_deref(), &mut session, &mut events, &stops, taking, reader)
                                .await?
                            {
                                break ended;
                            }
                        } else {
                            reader.error(THREAD_ENDED.to_string());
                        }
                    }
                    Wants::Frame(_) => match (pictures.as_mut(), &video) {
                        (Some(pictures), Ok(_)) => {
                            pictures.tell(Job::Ask).await;
                            reader.frame_asked();
                        }
                        (_, Err(why)) => reader.error(why.message()),
                        (None, Ok(_)) => reader.error(THREAD_ENDED.to_string()),
                    },
                },
            },
            event = on(told.map(mpsc::Receiver::recv)) => {
                if let Some(mut event) = event {
                    // WHY told here and not by the thread: the thread knows
                    // the frames, and the loop whose video they are.
                    if let (Event::Frame { of, .. }, Some(_)) = (&mut event, meeting.as_ref()) {
                        *of = crate::meeting::watching(reader);
                    }
                    if matches!(event, Event::Frame { asked: FrameAsked::Command, .. }) {
                        reader.frame.until = None;
                    }
                    reader.tell(&event);
                } else {
                    reader.error(THREAD_ENDED.to_string());
                    *pictures = None;
                }
            }
            came = on(meeting.as_mut().map(InMeeting::next)) => {
                if let Some(in_meeting) = meeting.as_mut()
                    && let Some(ended) = in_meeting.on(&mut session, reader, came).await
                {
                    break ended;
                }
            }
            came = on(video.as_mut().ok().map(Video::next)) => {
                if let Ok(going) = video.as_mut()
                    && let Err(why) = going.on(&mut session, came)
                {
                    let why = NoVideo::Failed(format!("given up: {why}"));
                    reader.error(why.message());
                    if let Ok(given_up) = std::mem::replace(&mut video, Err(why)) {
                        given_up.cancel();
                    }
                }
            }
            heard = on(room.as_mut().map(RoomSocket::next)) => match heard {
                Ok(broadcast) => {
                    // In a meeting the room's broadcast is nobody's of it,
                    // and the session stays.
                    if meeting.is_none() && broadcast.is_some() && broadcast != place.broadcast {
                        break Ended::Again;
                    }
                }
                // The room is gone; the chat goes on without it.
                Err(error) => {
                    reader.error(format!("the room is gone: {error:#}"));
                    *room = None;
                }
            },
            came = on(speech.as_mut().map(Follower::next)) => {
                if let Some(follower) = speech.as_mut() {
                    for out in follower.on(&mut session, came).await {
                        if let FollowOut::Item(piece) = &out {
                            crate::meeting::spoke(reader, &piece.from);
                        }
                        on_speech(reader, out);
                    }
                    remember_speech(reader, place, follower);
                }
            }
            came = tracks.next() => {
                for out in chat.tracks.on(&mut session, came).await {
                    match out {
                        Out::Line(line) => reader.tell(&chat_event(line)),
                        Out::Gap(gap) => reader.tell(&Event::ChatGap { after: gap.after, before: gap.before }),
                        Out::Note(note) => eprintln!("{note}"),
                    }
                }
                reader.had = chat.tracks.last_shown();
            }
            event = on(watching.as_mut().map(|watching| watching.reader.next_event())) => {
                if let Some(ended) = chat.on_watching(&mut session, reader, event) {
                    break ended;
                }
            }
            () = on(watch_at.map(tokio::time::sleep_until)) => {
                chat.watch(&mut session, connection, reader).await;
            }
            () = on(settle_at.map(tokio::time::sleep_until)) => {
                chat.settle_at = None;
                for event in reader.presence.settled() {
                    reader.tell(&event);
                }
            }
            event = asked.recv() => {
                let event = event.ok_or_else(|| anyhow!("the readers of request streams are gone"))?;
                if let Some(event) = chat.tracks.on_peer_request(&mut session, connection, event).await? {
                    session::refuse_peer_request(&mut session, &mut refused, event).await?;
                }
            }
            event = events.recv() => match event {
                Some(UniEvent::Object { header, object, .. })
                    if chat.tracks.lines_alias() == Some(header.track_alias) =>
                {
                    if let Some(line) = chat.tracks.live(header.group_id, &object.payload) {
                        reader.tell(&chat_event(line));
                    }
                    reader.had = chat.tracks.last_shown();
                }
                Some(UniEvent::Object { header, object, .. })
                    if speech.as_ref().and_then(Follower::track_alias) == Some(header.track_alias) =>
                {
                    if let Some(follower) = speech.as_mut() {
                        if let Some(piece) = follower.live(header.group_id, &object.payload) {
                            crate::meeting::spoke(reader, &piece.from);
                            on_speech(reader, FollowOut::Item(piece));
                        }
                        remember_speech(reader, place, follower);
                    }
                }
                Some(UniEvent::Object { header, object, .. })
                    if video.as_ref().ok().and_then(Video::track_alias) == Some(header.track_alias)
                        || meeting.as_ref().and_then(InMeeting::video_alias) == Some(header.track_alias) =>
                {
                    if let Some(pictures) = pictures.as_mut() {
                        on_video_object(pictures, reader, header.group_id, &object);
                    }
                }
                // An Object of the catalog's track, which stays subscribed:
                // a catalog that comes again is not read.
                Some(UniEvent::Object { .. } | UniEvent::SubgroupOpened { .. } | UniEvent::ObjectBegun { .. }) => {}
                Some(UniEvent::SubgroupClosed { header, .. }) => {
                    session.on_data_stream_closed(header.track_alias);
                }
                Some(UniEvent::FetchOpened { stream, header })
                    if chat.tracks.fetch_request_id() == Some(header.request_id) =>
                {
                    session.on_fetch_stream(header.request_id)?;
                    chat.tracks.read_fetch(stream, header);
                }
                Some(UniEvent::FetchOpened { stream, header })
                    if speech.as_ref().and_then(Follower::fetch_request_id) == Some(header.request_id) =>
                {
                    session.on_fetch_stream(header.request_id)?;
                    if let Some(follower) = speech.as_ref() {
                        follower.read_fetch(stream, header);
                    }
                }
                Some(UniEvent::FetchOpened { stream, header }) => {
                    session.on_fetch_stream(header.request_id)?;
                    drop(stops.take_fetch_body(stream));
                }
                Some(UniEvent::Failed(error)) => return Err(error.into()),
                // WHY the requests end at once, where §3.6 recommends
                // waiting: the chat would stand still while the old session
                // ends anyway, as in `xmoq-watch`.
                Some(UniEvent::Control(message)) => {
                    if let ControlEvent::GoAway(go_away) = session.on_control_message(message)? {
                        break Ended::Moved(new_url(&go_away));
                    }
                }
                Some(other) => bail!("not expected of the relay: {other:?}"),
                None => bail!("the connection is gone"),
            },
        }
    };
    if let Some(watching) = chat.watching.take() {
        watching.cancel();
    }
    if let Ok(video) = video {
        video.cancel();
    }
    if let Some(in_meeting) = meeting {
        in_meeting.cancel();
    }
    Ok(ended)
}
