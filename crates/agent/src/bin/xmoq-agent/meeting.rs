//! A meeting on one session (`--meeting`): who is in it, by one
//! `SUBSCRIBE_NAMESPACE` for `xmoq/meet/<room>`, and the video of the one
//! participant whose picture was asked for last. What is decided, and why
//! one video at a time, is `xmoq_viewer::meeting`'s; the sound is the
//! transcriber's, whose `speech` the session follows as for one broadcast.

use std::sync::Arc;
use std::time::Duration;

use xmoq_again::AskAgain;
use xmoq_agent::events::{Event, ParticipantState};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::session::{RequestEvent, Session};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{RequestReadError, RequestStreamEvent, StreamStops};
use xmoq_viewer::meeting::{self, Limits, Meeting, Video as VideoStep};
use xmoq_viewer::presence::{Change, SETTLE_MS};
use xmoq_viewer_io::session::{Events, GoneAway, new_url};

use crate::Reader;
use crate::attend::{Ended, Watching, on, watch};
use crate::frames::{Job, Pictures};
use crate::video::{Asking as VideoAsking, Came as VideoCame, Video};

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

/// The meeting of one session.
pub struct InMeeting {
    meeting: Meeting,
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
    /// What the meeting's clock counts from.
    began: tokio::time::Instant,
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
    /// The meeting of `--meeting`, on a session that has nobody of it yet,
    /// and asks for its namespaces; `None` without the flag.
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
            meeting::prefix(room)
                .into_iter()
                .map(String::into_bytes)
                .collect(),
        )?;
        let mut begun = Self {
            meeting: Meeting::new(room, limits),
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
            began: tokio::time::Instant::now(),
        };
        begun.watch(session, reader).await;
        eprintln!("XMOQ_AGENT_MEETING room={room}");
        Ok(Some(begun))
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX)
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

    /// A piece of the transcript was of the broadcast `from`.
    pub fn spoke(&mut self, from: &[String]) {
        self.meeting.spoke(from);
    }

    /// Whose video is watched, for the `id` of a `frame` line.
    pub fn watching(&self) -> Option<String> {
        self.meeting.watching().map(ToString::to_string)
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
    pub async fn on(
        &mut self,
        session: &mut Session,
        reader: &mut Reader,
        came: Came,
    ) -> Option<Ended> {
        match came {
            Came::Watching(event) => return self.on_watching(session, reader, event),
            Came::WatchNow => self.watch(session, reader).await,
            Came::Settled => {
                self.settle_at = None;
                let changes = self.meeting.settled();
                self.told(reader, changes);
            }
            Came::Linger => {
                self.linger_at = None;
                if let Some(id) = self.meeting.linger_over(self.now_ms()) {
                    eprintln!("XMOQ_AGENT_MEETING_VIDEO_LET_GO id={id}");
                    self.let_go_of_video();
                }
            }
            Came::Video(came) => {
                if let Some(video) = self.video.as_mut()
                    && let Err(why) = video.on(session, came)
                {
                    reader.error(format!("frame: the video is given up: {why}"));
                    self.meeting.video_failed();
                    self.let_go_of_video();
                }
            }
        }
        None
    }

    fn let_go_of_video(&mut self) {
        self.linger_at = None;
        if let Some(video) = self.video.take() {
            video.cancel();
        }
    }

    /// Writes the comings and goings, and lets go of a video whose
    /// participant left.
    fn told(&mut self, reader: &mut Reader, changes: impl IntoIterator<Item = Change>) {
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
        if self.video.is_some() && self.meeting.watching().is_none() {
            self.let_go_of_video();
        }
    }

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
                self.meeting.session_ended();
                self.let_go_of_video();
                self.watch_failed(reader, &format!("the relay closed it ({closed:?})"));
                return None;
            }
            // §10.18: a reset is as though each namespace got NAMESPACE_DONE;
            // those not told of again are taken to have left when the next
            // one settles.
            Err(error) => {
                session.cancel(request);
                self.meeting.session_ended();
                self.let_go_of_video();
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
                let change = fields(&namespace).and_then(|named| self.meeting.named(&named));
                self.told(reader, change);
            }
            Ok(RequestEvent::NamespaceDone(namespace)) => {
                let change = fields(&namespace).and_then(|named| self.meeting.done(&named));
                self.told(reader, change);
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
        let (id, step) = match self.meeting.asked(asked, self.now_ms()) {
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
                self.let_go_of_video();
            }
            let namespace = match TrackNamespace::new(
                meeting::prefix(&self.room)
                    .into_iter()
                    .chain(std::iter::once(id.clone()))
                    .map(String::into_bytes)
                    .collect(),
            ) {
                Ok(namespace) => namespace,
                Err(error) => {
                    reader.error(format!("frame: {id}: {error}"));
                    self.meeting.video_failed();
                    return Ok(None);
                }
            };
            let asking_with = VideoAsking {
                connection: Arc::clone(&self.connection),
                namespace,
                token: self.token.clone(),
                answer_wait_ms: self.answer_wait_ms,
                open_wait_ms: self.open_wait_ms,
            };
            // The catalog's SUBSCRIBE and FETCH, the catalog itself, and the
            // video's SUBSCRIBE, as for one broadcast.
            let wait = Duration::from_millis(self.answer_wait_ms * 3) + Duration::from_secs(5);
            let begun =
                tokio::time::timeout(wait, Video::begin(asking_with, session, events, stops)).await;
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
                    if error
                        .chain()
                        .any(<dyn std::error::Error>::is::<xmoq_moqt::Error>)
                    {
                        return Err(error);
                    }
                    reader.error(format!("frame: no video of {id}: {error:#}"));
                    self.meeting.video_failed();
                    self.linger_at = None;
                    return Ok(None);
                }
                Err(_) => {
                    reader.error(format!(
                        "frame: no video of {id} within {} ms",
                        wait.as_millis()
                    ));
                    self.meeting.video_failed();
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
    pub fn cancel(self) {
        if let Some(watching) = self.watching {
            watching.cancel();
        }
        if let Some(video) = self.video {
            video.cancel();
        }
    }
}
