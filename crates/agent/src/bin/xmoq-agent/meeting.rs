//! A meeting on one session (`--meeting`): who is in it, by one
//! `SUBSCRIBE_NAMESPACE` for `xmoq/meet/<room>`, and the video of the one
//! participant whose picture was asked for last. The names are
//! `xmoq_meeting_tracks`'; what is decided, and why one video at a time, is
//! `xmoq_viewer::meeting`'s; the sound is the transcriber's, whose `speech`
//! the session follows as for one broadcast.

use std::sync::Arc;
use std::time::Duration;

use xmoq_again::AskAgain;
use xmoq_agent::events::{Event, ParticipantState};
use xmoq_moqt::RequestFault;
use xmoq_moqt::message::request_error_code;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{RequestEvent, RequestId, Session};
use xmoq_moqt_io::client;
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{RequestReadError, RequestStreamEvent, StreamStops};
use xmoq_subscriber::kept::{Asking, Came as VideoCame};
use xmoq_subscriber::session::{Events, GoingAway, GoneAway, new_url};
use xmoq_viewer::meeting::{Limits, Meeting, Video as VideoStep};
use xmoq_viewer::presence::{Change, SETTLE_MS};

use crate::Reader;
use crate::attend::{Ended, Watching, on, watch};
use crate::frames::{Job, Pictures};
use crate::video::Video;

/// What the meeting has to be told of, from the loop of the session.
pub enum Came {
    /// What came on the stream of the `SUBSCRIBE_NAMESPACE`.
    Watching(Result<RequestStreamEvent, RequestReadError>),
    /// The time to ask for the namespaces (again) has come.
    WatchNow,
    /// Those not told of again on this session have left.
    Settled,
    /// No picture of the watched participant was asked for in a while.
    Linger,
    /// What came of the video.
    Video(VideoCame),
}

/// Who is in the meeting, and whose voice was read last, from one session
/// to the next, as [`crate::Reader`]'s `presence` is for the chat.
///
/// WHY kept over the sessions and not made anew with each: on a new
/// session the relay tells of everybody there again, and a meeting made
/// anew would write each as joined once more, and never as left one who
/// went meanwhile (`xmoq_viewer::meeting`, as `presence`).
pub struct Kept {
    meeting: Meeting,
    /// What the meeting's clock counts from.
    began: tokio::time::Instant,
}

impl Kept {
    /// The meeting of `--meeting`; `None` without the flag.
    #[must_use]
    pub fn of(args: &xmoq_agent::args::Args) -> Option<Self> {
        let (room, limits) = args.meeting.as_ref()?;
        Some(Self {
            meeting: Meeting::new(room, *limits),
            began: tokio::time::Instant::now(),
        })
    }

    /// The session ended: those the next one does not tell of again within
    /// `SETTLE_MS` have left, and no video is subscribed.
    pub fn session_ended(&mut self) {
        self.meeting.session_ended();
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// A piece of the transcript was of the broadcast `from`.
pub fn spoke(reader: &mut Reader, from: &[String]) {
    if let Some(kept) = reader.meeting.as_mut() {
        kept.meeting.spoke(from);
    }
}

/// The video asked for could not be had: none is subscribed.
fn video_failed(reader: &mut Reader) {
    if let Some(kept) = reader.meeting.as_mut() {
        kept.meeting.video_failed();
    }
}

/// The `SUBSCRIBE_NAMESPACE` ended within the session: as for a session
/// that ended.
fn session_ended(reader: &mut Reader) {
    if let Some(kept) = reader.meeting.as_mut() {
        kept.session_ended();
    }
}

/// Whose video is watched, for the `id` of a `frame` line.
#[must_use]
pub fn watching(reader: &Reader) -> Option<String> {
    reader
        .meeting
        .as_ref()
        .and_then(|kept| kept.meeting.watching())
        .map(ToString::to_string)
}

/// The meeting of one session: its `SUBSCRIBE_NAMESPACE` and its video.
pub struct InMeeting {
    room: String,
    prefix: TrackNamespace,
    limits: Limits,
    connection: Arc<WtConnection>,
    /// The session's token, where the room gave one.
    token: Option<Vec<u8>>,
    answer_wait_ms: u64,
    open_wait_ms: u64,
    watching: Option<Watching>,
    watch_at: Option<tokio::time::Instant>,
    again: AskAgain,
    settle_at: Option<tokio::time::Instant>,
    video: Option<Video>,
    linger_at: Option<tokio::time::Instant>,
}

/// A namespace's fields as text, as the meeting reads them; `None` for one
/// that is not UTF-8, which is nobody's of the meeting.
fn fields(namespace: &TrackNamespace) -> Option<Vec<String>> {
    namespace
        .fields()
        .iter()
        .map(|field| String::from_utf8(field.clone()).ok())
        .collect()
}

impl InMeeting {
    /// The meeting of `--meeting` on a new session, which asks for its
    /// namespaces; `None` without the flag. Who is in it is the reader's
    /// ([`Kept`]).
    ///
    /// # Errors
    ///
    /// Where the meeting's prefix is no namespace (§2.4.1).
    pub async fn begin(
        args: &xmoq_agent::args::Args,
        connection: &Arc<WtConnection>,
        token: Option<Vec<u8>>,
        session: &mut Session,
        reader: &mut Reader,
    ) -> anyhow::Result<Option<Self>> {
        let Some((room, limits)) = &args.meeting else {
            return Ok(None);
        };
        let (room, limits) = (room.as_str(), *limits);
        let (answer_wait_ms, open_wait_ms) = (args.answer_wait_ms, args.open_wait_ms);
        let prefix = TrackNamespace::new(
            xmoq_meeting_tracks::prefix(room)
                .into_iter()
                .map(String::into_bytes)
                .collect(),
        )?;
        let mut begun = Self {
            room: room.to_string(),
            prefix,
            limits,
            connection: Arc::clone(connection),
            token,
            answer_wait_ms,
            open_wait_ms,
            watching: None,
            watch_at: None,
            again: AskAgain::default(),
            settle_at: None,
            video: None,
            linger_at: None,
        };
        begun.watch(session, reader).await;
        eprintln!("XMOQ_AGENT_MEETING room={room}");
        Ok(Some(begun))
    }

    async fn watch(&mut self, session: &mut Session, reader: &mut Reader) {
        self.watch_at = None;
        match watch(
            &self.connection,
            session,
            &self.prefix,
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
            "who is in the meeting: {how}; asking again in {pause} ms"
        ));
    }

    /// The Track Alias of the watched participant's video.
    pub fn video_alias(&self) -> Option<u64> {
        self.video.as_ref().and_then(Video::track_alias)
    }

    /// What comes next.
    pub async fn next(&mut self) -> Came {
        let Self {
            watching,
            watch_at,
            settle_at,
            linger_at,
            video,
            ..
        } = self;
        tokio::select! {
            event = on(watching.as_mut().map(|watching| watching.reader.next_event())) => Came::Watching(event),
            () = on(watch_at.map(tokio::time::sleep_until)) => Came::WatchNow,
            () = on(settle_at.map(tokio::time::sleep_until)) => Came::Settled,
            () = on(linger_at.map(tokio::time::sleep_until)) => Came::Linger,
            came = on(video.as_mut().map(Video::next)) => Came::Video(came),
        }
    }

    /// What came. `Some` where the session is to end for it.
    ///
    /// # Errors
    ///
    /// A message on the video's stream that breaks the protocol: the
    /// session is to be closed with its code.
    pub async fn on(
        &mut self,
        session: &mut Session,
        reader: &mut Reader,
        came: Came,
    ) -> Result<Option<Ended>, RequestFault> {
        match came {
            Came::Watching(event) => return Ok(self.on_watching(session, reader, event)),
            Came::WatchNow => self.watch(session, reader).await,
            Came::Settled => {
                self.settle_at = None;
                let changes = reader
                    .meeting
                    .as_mut()
                    .map(|kept| kept.meeting.settled())
                    .unwrap_or_default();
                self.told(session, reader, changes);
            }
            Came::Linger => {
                self.linger_at = None;
                let over = reader.meeting.as_mut().and_then(|kept| {
                    let now_ms = kept.now_ms();
                    kept.meeting.linger_over(now_ms)
                });
                if let Some(id) = over {
                    eprintln!("XMOQ_AGENT_MEETING_VIDEO_LET_GO id={id}");
                    self.let_go_of_video(session);
                }
            }
            Came::Video(came) => {
                if let Some(video) = self.video.as_mut()
                    && let Some(why) = video.on(session, came)?
                {
                    reader.error(format!("frame: the video is given up: {why}"));
                    video_failed(reader);
                    self.let_go_of_video(session);
                }
            }
        }
        Ok(None)
    }

    /// The session ended `request` for a malformed track: whether it was of
    /// the video of the participant watched ([`Video::malformed`]).
    pub fn malformed(&mut self, request: RequestId) -> bool {
        self.video
            .as_mut()
            .is_some_and(|video| video.malformed(request))
    }

    fn let_go_of_video(&mut self, session: &mut Session) {
        self.linger_at = None;
        if let Some(video) = self.video.take() {
            video.cancel(session);
        }
    }

    /// Writes the comings and goings, and lets go of a video whose
    /// participant left.
    fn told(
        &mut self,
        session: &mut Session,
        reader: &mut Reader,
        changes: impl IntoIterator<Item = Change>,
    ) {
        for change in changes {
            match change {
                Change::Came(id) => reader.tell(&Event::Participant {
                    state: ParticipantState::Joined,
                    id,
                }),
                Change::Left(id) => reader.tell(&Event::Participant {
                    state: ParticipantState::Left,
                    id,
                }),
                Change::TooMany(id) => reader.error(format!(
                    "who is in the meeting: {id} is left out, {} are kept",
                    self.limits.most_participants
                )),
            }
        }
        if self.video.is_some() && watching(reader).is_none() {
            self.let_go_of_video(session);
        }
    }

    /// FIXME: one keeping of a `SUBSCRIBE_NAMESPACE` for this and for the
    /// other (`attend::Chat::on_watching`), as `xmoq_subscriber::kept` is for a track, where only
    /// more namespaces than the session keeps ask again, and any other
    /// message that breaks the protocol closes the session.
    fn on_watching(
        &mut self,
        session: &mut Session,
        reader: &mut Reader,
        event: Result<RequestStreamEvent, RequestReadError>,
    ) -> Option<Ended> {
        let request = self.watching.as_ref().map(|watching| watching.request)?;
        let message = match event {
            Ok(RequestStreamEvent::Message(message)) => message,
            Ok(RequestStreamEvent::Closed(closed)) => {
                session.cancel(request);
                session_ended(reader);
                self.let_go_of_video(session);
                self.watch_failed(reader, &format!("the relay closed it ({closed:?})"));
                return None;
            }
            // §10.18: a reset is as though each namespace got NAMESPACE_DONE;
            // those not told of again are taken to have left when the next
            // one settles.
            Err(error) => {
                session.cancel(request);
                session_ended(reader);
                self.let_go_of_video(session);
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
                let change = fields(&namespace).and_then(|named| {
                    reader
                        .meeting
                        .as_mut()
                        .and_then(|kept| kept.meeting.named(&named))
                });
                self.told(session, reader, change);
            }
            Ok(RequestEvent::NamespaceDone(namespace)) => {
                let change = fields(&namespace).and_then(|named| {
                    reader
                        .meeting
                        .as_mut()
                        .and_then(|kept| kept.meeting.done(&named))
                });
                self.told(session, reader, change);
            }
            Ok(RequestEvent::Rejected(error)) => {
                session.cancel(request);
                self.watch_failed(reader, &format!("refused with {:#x}", error.error_code));
            }
            Ok(RequestEvent::GoAway(go_away)) => return Some(Ended::Moved(new_url(&go_away))),
            Ok(other) => reader.error(format!("who is in the meeting: {other:?}")),
            Err(error) => {
                session.cancel(request);
                self.watch_failed(reader, &format!("{error}"));
            }
        }
        None
    }

    /// `/frame` or `/frame <id>`: the picture of whom it names, from the
    /// video subscribed to for it. `Some` where the session is to end.
    ///
    /// # Errors
    ///
    /// An error of the session while its video was begun, which closes the
    /// session, as for one broadcast.
    pub async fn frame(
        &mut self,
        asked: Option<&str>,
        session: &mut Session,
        events: &mut Events,
        stops: &StreamStops,
        pictures: &mut Pictures,
        reader: &mut Reader,
    ) -> anyhow::Result<Option<Ended>> {
        let Some(kept) = reader.meeting.as_mut() else {
            return Ok(None);
        };
        let now_ms = kept.now_ms();
        let (id, step) = match kept.meeting.asked(asked, now_ms) {
            Ok(target) => target,
            Err(why) => {
                reader.error(format!("frame: {why}"));
                return Ok(None);
            }
        };
        self.linger_at =
            Some(tokio::time::Instant::now() + Duration::from_millis(self.limits.video_linger_ms));
        if let VideoStep::Subscribe { cancel, .. } = &step {
            if cancel.is_some() {
                self.let_go_of_video(session);
            }
            let namespace = match TrackNamespace::new(
                xmoq_meeting_tracks::participant(&self.room, &id)
                    .into_iter()
                    .map(String::into_bytes)
                    .collect(),
            ) {
                Ok(namespace) => namespace,
                Err(error) => {
                    reader.error(format!("frame: {id}: {error}"));
                    video_failed(reader);
                    return Ok(None);
                }
            };
            let asking_with = Asking {
                connection: Arc::clone(&self.connection),
                stops: stops.clone(),
                token: self.token.clone(),
                answer_wait_ms: self.answer_wait_ms,
                open_wait_ms: self.open_wait_ms,
            };
            // The catalog's SUBSCRIBE and FETCH, the catalog itself, and the
            // video's SUBSCRIBE, as for one broadcast.
            let wait = Duration::from_millis(self.answer_wait_ms * 3) + Duration::from_secs(5);
            let begun =
                tokio::time::timeout(wait, Video::begin(asking_with, namespace, session, events))
                    .await;
            match begun {
                Ok(Ok(video)) => {
                    eprintln!("XMOQ_AGENT_MEETING_VIDEO id={id}");
                    self.video = Some(video);
                    // Before the first Object of this video reaches the
                    // thread: they go through the loop.
                    pictures.tell(Job::BeginAgain).await;
                }
                Ok(Err(error)) => {
                    if let Some(GoneAway(go_away)) =
                        error.chain().find_map(|cause| cause.downcast_ref())
                    {
                        return Ok(Some(Ended::Moved(new_url(go_away))));
                    }
                    // §10.4: as for one broadcast (`attend`).
                    if error.chain().any(<dyn std::error::Error>::is::<GoingAway>) {
                        let waiting = async {
                            Ok(client::go_away_behind(
                                session,
                                events,
                                request_error_code::GOING_AWAY,
                                Duration::from_millis(self.answer_wait_ms),
                            )
                            .await?)
                        };
                        let wait = Duration::from_millis(self.answer_wait_ms) * 2;
                        match reader.unless_ended("the GOAWAY", wait, waiting).await? {
                            None => return Ok(Some(Ended::Quit)),
                            Some(Some(go_away)) => {
                                return Ok(Some(Ended::Moved(new_url(&go_away))));
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
                    reader.error(format!("frame: no video of {id}: {error:#}"));
                    video_failed(reader);
                    self.linger_at = None;
                    return Ok(None);
                }
                Err(_) => {
                    reader.error(format!(
                        "frame: no video of {id} within {} ms",
                        wait.as_millis()
                    ));
                    video_failed(reader);
                    self.linger_at = None;
                    return Ok(None);
                }
            }
        }
        pictures.tell(Job::Ask).await;
        reader.frame_asked();
        Ok(None)
    }

    /// The session ended: the watch and the video go with it.
    pub fn cancel(self, session: &mut Session) {
        if let Some(watching) = self.watching {
            watching.cancel();
        }
        if let Some(video) = self.video {
            video.cancel(session);
        }
    }
}
