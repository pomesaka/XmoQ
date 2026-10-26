//! One session at a broadcast: the chat on tracks, who is in it, the lines
//! of stdin said in it, and the pictures of the video asked for.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use tokio::sync::mpsc;
pub use xmoq_again::next_session::Ended;
use xmoq_again::next_session::Pause;
use xmoq_agent::events::{BroadcastState, Event, FrameReason};
use xmoq_agent::input::{self, Input};
use xmoq_agent::presence::SETTLE_MS;
use xmoq_chat_tracks::names::{person_of as person_of_namespace, room_namespace};
use xmoq_chat_tracks::payloads::Line as ChatLine;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::{ObjectStatus, SubgroupObject};
use xmoq_moqt::message::request_error_code;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::Parameter;
use xmoq_moqt::session::{
    ControlEvent, FetchStreamOpened, MalformedRequest, PeerRequestId, RequestEvent, RequestId,
    Role, Session, Transport,
};
use xmoq_moqt::stream_reset_code;
use xmoq_moqt_io::client::{self, UntilClosed};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, RequestReader, RequestStreamEvent, SendStream, StreamConfig, StreamStops, Trace,
    UniEvent, read_request_streams, read_uni_streams, send_message,
};
use xmoq_msf::CatalogTrack;
use xmoq_subscriber::catalog::{Became, FollowedCatalog};
use xmoq_subscriber::follow::{Asking, Follower, Out as FollowOut, RENDEZVOUS_MS, Track};
use xmoq_subscriber::namespaces::KeptNamespace;
use xmoq_subscriber::session::{
    self, Events, GoingAway, GoneAway, Requests, new_url, timestamp_of,
};
use xmoq_transcript_tracks::names::{SPEECH, transcript_namespace};
use xmoq_transcript_tracks::payloads::{Speech, read_speech};
use xmoq_viewer_io::chat_tracks::{ChatOutcome, ChatTracks};
use xmoq_viewer_io::room_client::{HeldSeat, SeatLost};

use crate::frames::{Job, Pictures};
use crate::meeting::InMeeting;
use crate::stdin::Line;
use crate::video::Video;
use crate::{Outside, Place, Reader, person_of};

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

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
    /// The `sub` token of the room's namespace, where the room gave one.
    token: Option<Vec<u8>>,
    open_wait_ms: u64,
    watching: Option<Watching>,
    /// The `SUBSCRIBE_NAMESPACE` for the namespaces, and when it is asked
    /// for again (`xmoq-subscriber`).
    kept: KeptNamespace,
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
        self.kept.asked(std::time::Instant::now());
        match watch(
            connection,
            session,
            self.kept.prefix(),
            self.token.as_deref(),
            self.open_wait_ms,
        )
        .await
        {
            Ok(watching) => self.watching = Some(watching),
            Err(error) => self.watch_failed(reader, &format!("{error:#}")),
        }
    }

    /// The `SUBSCRIBE_NAMESPACE` ended, or could not be sent: asked for again
    /// after the role's pause.
    fn watch_failed(&mut self, reader: &mut Reader, how: &str) {
        let pause = self.kept.ended(std::time::Instant::now());
        self.watch_again_in(reader, how, pause);
    }

    /// The `SUBSCRIBE_NAMESPACE` was refused: asked for again after the wait
    /// its Retry Interval names (`named_ms`).
    fn watch_refused(&mut self, reader: &mut Reader, how: &str, named_ms: Option<u64>) {
        let pause = self.kept.refused(named_ms, std::time::Instant::now());
        self.watch_again_in(reader, how, pause);
    }

    fn watch_again_in(&mut self, reader: &mut Reader, how: &str, pause: u64) {
        if let Some(watching) = self.watching.take() {
            watching.cancel();
        }
        reader.error(format!(
            "who is in the chat: {how}; asking again in {pause} ms"
        ));
    }

    /// What came on the stream of the `SUBSCRIBE_NAMESPACE`.
    ///
    /// FIXME: one reading of the stream of a `SUBSCRIBE_NAMESPACE` for this
    /// and for the other (`meeting::InMeeting::on_watching`), where only more namespaces than the
    /// session keeps ask again, and any other message that breaks the
    /// protocol closes the session. When it is asked again is
    /// `KeptNamespace` of `xmoq-subscriber` already.
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
                self.kept.subscribed();
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
                let how = format!("refused with {:#x}", error.error_code);
                self.watch_refused(reader, &how, error.retry_wait_ms());
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
    /// The session is in a meeting, whose participants' pictures are
    /// `InMeeting`'s; or the broadcast's catalog has not come (yet).
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

/// The most an Object of `speech` may be. **Provisional**: a piece of a few
/// seconds is some hundred bytes of text.
const SPEECH_PAYLOAD_MOST: usize = 64 * 1024;

/// The most bytes of the stream of the FETCH of `speech`: the pieces
/// fetched, with room. **Provisional.**
const SPEECH_FETCH_MOST: usize = 1024 * 1024;

/// How many of the pieces missed while the subscription was away are
/// fetched: as many as of the chat's lines by default. **Provisional.**
const SPEECH_CATCH_UP: u64 = xmoq_viewer::chat_track::CATCH_UP_LINES;

/// Writes what came of the transcript; a piece also tells the meeting who
/// spoke.
fn on_speech(reader: &mut Reader, out: FollowOut<Speech>) {
    match out {
        FollowOut::Item(piece) => {
            crate::meeting::spoke(reader, &piece.from);
            reader.tell(&Event::Speech {
                number: piece.number,
                from: piece.from,
                text: piece.text,
                start_us: piece.start_us,
                end_us: piece.end_us,
                read_at: piece.at,
            });
        }
        FollowOut::Gap(gap) => reader.error(format!(
            "speech: {} pieces not fetched ({} to {})",
            gap.before - gap.after - 1,
            gap.after + 1,
            gap.before - 1
        )),
        FollowOut::Note(note) => eprintln!("{note}"),
    }
}

/// Writes what came of the chat on tracks.
fn on_chat(reader: &mut Reader, out: ChatOutcome) {
    match out {
        ChatOutcome::Line(line) => reader.tell(&chat_event(line)),
        ChatOutcome::Gap(gap) => reader.tell(&Event::ChatGap {
            after: gap.after,
            before: gap.before,
        }),
        ChatOutcome::Note(note) => eprintln!("{note}"),
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

/// What the requests of a track of the session are sent with.
fn asking_with(
    args: &xmoq_agent::args::Args,
    connection: &Arc<WtConnection>,
    stops: &StreamStops,
    token: Option<Vec<u8>>,
) -> Asking {
    Asking {
        connection: Arc::clone(connection),
        stops: stops.clone(),
        token,
        answer_wait_ms: args.answer_wait_ms,
        open_wait_ms: args.open_wait_ms,
    }
}

/// The broadcast's catalog ended within the session: written, and its
/// video given up, to be asked for when the catalog comes again.
fn broadcast_ended(
    reader: &mut Reader,
    place: &Place,
    video: &mut Result<Video, NoVideo>,
    session: &mut Session,
    frames: bool,
) {
    if std::mem::take(&mut reader.broadcast_began) {
        crate::tell_broadcast(reader, place, BroadcastState::Ended);
    }
    let none = if frames {
        NoVideo::NoBroadcast
    } else {
        NoVideo::Off
    };
    if let Ok(ended) = std::mem::replace(video, Err(none)) {
        ended.cancel(session);
    }
}

/// What came of subscribing to the broadcast's video.
enum Started {
    /// Boxed: it is the largest by far.
    Video(Box<Video>),
    /// None on this session, for this, which is told.
    No(NoVideo),
    /// The relay is going away: the session ends so.
    SessionEnds(Ended),
}

/// Subscribes to the video of `tracks`, the broadcast's catalog, that came
/// on the session.
#[expect(
    clippy::too_many_arguments,
    reason = "the parts of the session the loop holds apart, borrowed for one call"
)]
async fn start_video(
    args: &xmoq_agent::args::Args,
    connection: &Arc<WtConnection>,
    stops: &StreamStops,
    place: &Place,
    tracks: &[CatalogTrack],
    session: &mut Session,
    events: &mut Events,
    pictures: &mut Pictures,
    reader: &mut Reader,
) -> Result<Started> {
    let Some(namespace) = &place.namespace else {
        return Ok(Started::No(NoVideo::NoBroadcast));
    };
    let namespace = TrackNamespace::new(
        namespace
            .iter()
            .map(|field| field.as_bytes().to_vec())
            .collect(),
    )?;
    let asking = asking_with(args, connection, stops, place.token.clone());
    let subscribed = Video::of(asking, namespace, tracks, None, session, events);
    let wait = Duration::from_millis(args.answer_wait_ms) * 2;
    match reader.unless_ended("the video", wait, subscribed).await {
        Ok(None) => Ok(Started::SessionEnds(Ended::Quit)),
        Ok(Some(video)) => {
            // Before the first Object of this video reaches the thread: the
            // Objects go through the loop.
            pictures.tell(Job::BeginAgain).await;
            Ok(Started::Video(Box::new(video)))
        }
        Err(error) => {
            if let Some(GoneAway(go_away)) = error.chain().find_map(|cause| cause.downcast_ref()) {
                return Ok(Started::SessionEnds(Ended::Moved(new_url(go_away))));
            }
            // §10.4: a relay going away refuses with `GOING_AWAY`, and its
            // GOAWAY, on the control stream, can come after.
            // WHY `unless_ended`: a signal and stdin are heard meanwhile.
            if error.chain().any(<dyn std::error::Error>::is::<GoingAway>) {
                let waiting = async {
                    Ok(client::go_away_behind(
                        session,
                        events,
                        request_error_code::GOING_AWAY,
                        Duration::from_millis(args.answer_wait_ms),
                    )
                    .await?)
                };
                let wait = Duration::from_millis(args.answer_wait_ms) * 2;
                match reader.unless_ended("the GOAWAY", wait, waiting).await? {
                    None => return Ok(Started::SessionEnds(Ended::Quit)),
                    Some(Some(go_away)) => {
                        return Ok(Started::SessionEnds(Ended::Moved(new_url(&go_away))));
                    }
                    Some(None) => {}
                }
            }
            if error
                .chain()
                .any(<dyn std::error::Error>::is::<xmoq_moqt::Error>)
            {
                return Err(error);
            }
            let why = NoVideo::Failed(format!("{error:#}"));
            reader.error(why.message());
            Ok(Started::No(why))
        }
    }
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
    seat: &mut Option<HeldSeat>,
    reader: &mut Reader,
    pictures: &mut Option<Pictures>,
    kept: &mut UntilClosed<WtConnection>,
) -> Result<Ended> {
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let stops = StreamStops::default();
    let (queue, received) = mpsc::channel(64);
    let mut events = Events::new(received);
    kept.readers.push(read_uni_streams(
        Arc::clone(connection),
        StreamConfig::default(),
        stops.clone(),
        queue,
        |event| event,
    ));
    let (asks, mut asked) = mpsc::channel(16);
    kept.readers.push(read_request_streams(
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
    // In a meeting, and until the broadcast's catalog comes.
    let mut video = Err(if pictures.is_some() {
        NoVideo::NoBroadcast
    } else {
        NoVideo::Off
    });
    // WHY the catalog is followed also with `--frames off`: it says when the
    // broadcast begins and ends (`broadcast` lines). WHY on the loop and not
    // waited for here: the room names its broadcast before it begins, and
    // the chat goes on meanwhile.
    let mut catalog = match &place.namespace {
        Some(namespace) => Some(FollowedCatalog::new(
            TrackNamespace::new(
                namespace
                    .iter()
                    .map(|field| field.as_bytes().to_vec())
                    .collect(),
            )?,
            place.catalog_rendezvous_ms,
            asking_with(args, connection, &stops, place.token.clone()),
        )),
        None => None,
    };
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
        token: place.chat_tokens.subscribe.clone(),
        open_wait_ms: args.open_wait_ms,
        watching: None,
        kept: KeptNamespace::new(room_namespace, Pause::VIEWER),
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
                    // WHY held: a transcriber that starts after the reader
                    // has its pieces read from the start.
                    rendezvous_ms: Some(RENDEZVOUS_MS),
                    payload_most: SPEECH_PAYLOAD_MOST,
                    fetch_most: SPEECH_FETCH_MOST,
                    recent: xmoq_viewer::chat_track::RECENT_LINES,
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
            kept,
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
                    if matches!(event, Event::Frame { asked: FrameReason::Command, .. }) {
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
                    && let Some(ended) = in_meeting.on(&mut session, reader, came).await?
                {
                    break ended;
                }
            }
            came = on(catalog.as_mut().map(FollowedCatalog::next)) => {
                let Some(followed) = catalog.as_mut() else { continue };
                // WHY under `unless_ended`: where the SUBSCRIBE was answered,
                // `on` reads the catalog (a stream for its FETCH, then 5 s at
                // most), and a signal and `--seconds` are heard meanwhile.
                let reading = followed.on(&mut session, &mut events, came);
                let wait = Duration::from_millis(args.open_wait_ms + args.answer_wait_ms)
                    + Duration::from_secs(5);
                let became = match reader.unless_ended("the catalog", wait, reading).await {
                    Ok(Some(became)) => became,
                    Ok(None) => break Ended::Quit,
                    Err(error) => {
                        if let Some(GoneAway(go_away)) = error.chain().find_map(|cause| cause.downcast_ref()) {
                            break Ended::Moved(new_url(go_away));
                        }
                        return Err(error);
                    }
                };
                match became {
                    Became::Began(tracks) => {
                        reader.broadcast_began = true;
                        crate::tell_broadcast(reader, place, BroadcastState::Began);
                        if let Some(taking) = pictures.as_mut() {
                            match start_video(args, connection, &stops, place, &tracks, &mut session, &mut events, taking, reader).await? {
                                Started::Video(started) => video = Ok(*started),
                                Started::No(why) => video = Err(why),
                                Started::SessionEnds(ended) => break ended,
                            }
                        }
                    }
                    Became::Ended(how) => {
                        eprintln!("XMOQ_AGENT_CATALOG_ENDED {how}");
                        broadcast_ended(reader, place, &mut video, &mut session, pictures.is_some());
                    }
                    Became::Waiting { why, pause_ms } => {
                        eprintln!("XMOQ_AGENT_CATALOG_WAITING {why}; asking again in {pause_ms} ms");
                    }
                    Became::Nothing => {}
                }
            }
            came = on(video.as_mut().ok().map(Video::next)) => {
                if let Ok(going) = video.as_mut()
                    && let Some(why) = going.on(&mut session, came)?
                {
                    let why = NoVideo::Failed(format!("given up: {why}"));
                    reader.error(why.message());
                    if let Ok(given_up) = std::mem::replace(&mut video, Err(why)) {
                        given_up.cancel(&mut session);
                    }
                }
            }
            lost = on(seat.as_ref().map(HeldSeat::lost)) => {
                // WHY the session ends: its token is the seat's, and the
                // next session takes a seat again.
                let why = match lost {
                    SeatLost::Gone => "it ran out".to_string(),
                    SeatLost::Failed(why) => why,
                };
                reader.error(format!("the seat in the meeting was lost: {why}; taking one again"));
                *seat = None;
                break Ended::Again;
            }
            came = on(speech.as_mut().map(Follower::next)) => {
                if let Some(follower) = speech.as_mut() {
                    for out in follower.on(&mut session, came).await? {
                        on_speech(reader, out);
                    }
                    remember_speech(reader, place, follower);
                }
            }
            came = tracks.next() => {
                for out in chat.tracks.on(&mut session, came).await? {
                    on_chat(reader, out);
                }
                reader.had = chat.tracks.last_shown();
            }
            event = on(watching.as_mut().map(|watching| watching.reader.next_event())) => {
                if let Some(ended) = chat.on_watching(&mut session, reader, event) {
                    break ended;
                }
            }
            () = on(kept.again_at().map(|at| tokio::time::sleep_until(at.into()))) => {
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
            event = events.recv_sorted(&session, &stops) => match event {
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
                    if let Some(follower) = speech.as_mut() {
                        follower.read_fetch(stream, header);
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
                Some(UniEvent::Failed(error)) => return Err(error.into()),
                // WHY the requests end at once, where §3.6 recommends
                // waiting: the chat would stand still while the old session
                // ends anyway, as in `xmoq-watch`.
                Some(UniEvent::Control(message)) => {
                    if let ControlEvent::GoAway(go_away) = session.on_control_message(message)? {
                        break Ended::Moved(new_url(&go_away));
                    }
                }
                // §2.4.2: "it MUST cancel any corresponding subscription or
                // fetches for that Track". Each that was of it is asked for
                // again on the session, as after any end of it.
                Some(UniEvent::TrackMalformed { header, error, .. }) => {
                    eprintln!("XMOQ_AGENT_TRACK_MALFORMED track_alias={} error={error}", header.track_alias);
                    events.stop_streams_of(header.track_alias, &stops);
                    for cancelled in session.cancel_malformed(header.track_alias) {
                        // The reader takes no PUBLISH of the relay.
                        let (MalformedRequest::Subscribe(request) | MalformedRequest::Fetch(request)) = cancelled else {
                            continue;
                        };
                        if video.as_mut().is_ok_and(|video| video.malformed(request))
                            || meeting.as_mut().is_some_and(|in_meeting| in_meeting.malformed(request))
                        {
                            continue;
                        }
                        if let Some(became) = catalog.as_mut().and_then(|followed| followed.malformed(request)) {
                            eprintln!("XMOQ_AGENT_CATALOG_MALFORMED");
                            if let Became::Ended(_) = became {
                                broadcast_ended(reader, place, &mut video, &mut session, pictures.is_some());
                            }
                            continue;
                        }
                        if let Some(outs) = chat.tracks.malformed(request) {
                            for out in outs {
                                on_chat(reader, out);
                            }
                            reader.had = chat.tracks.last_shown();
                        } else if let Some(outs) = speech.as_mut().and_then(|follower| follower.malformed(request)) {
                            for out in outs {
                                on_speech(reader, out);
                            }
                            if let Some(follower) = speech.as_ref() {
                                remember_speech(reader, place, follower);
                            }
                        } else {
                            eprintln!("XMOQ_AGENT_TRACK_MALFORMED_UNCLAIMED request_id={}", request.value());
                        }
                    }
                }
                // The fault is on this side: INTERNAL_ERROR, as every client.
                Some(UniEvent::ReaderPanicked) => {
                    return Err(xmoq_moqt_io::client::ReaderPanicked.into());
                }
                None => bail!("the connection is gone"),
            },
        }
    };
    if let Some(watching) = chat.watching.take() {
        watching.cancel();
    }
    if let Ok(video) = video {
        video.cancel(&mut session);
    }
    if let Some(followed) = catalog {
        followed.cancel(&mut session);
    }
    if let Some(in_meeting) = meeting {
        in_meeting.cancel(&mut session);
    }
    Ok(ended)
}
