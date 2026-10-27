//! The chat on `MoQ` tracks, on the session of the broadcast
//! (`xmoq_viewer::chat_track` decides): the viewer announces its namespace
//! and publishes `said` to the gatherer that subscribes to it, and follows
//! the room's `lines` (`xmoq_subscriber::follow`). Nothing of it ends the session:
//! what fails is said, and the broadcast plays on.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use xmoq_chat_tracks::limits::TEXT_BYTES;
use xmoq_chat_tracks::names::{LINES, Person, SAID, person_namespace, room_namespace};
use xmoq_chat_tracks::payloads::{Line, SentLine, read_line};
use xmoq_moqt::RequestFault;
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::name::FullTrackName;
use xmoq_moqt::params::Parameter;
use xmoq_moqt::session::{
    PeerRequest, PeerRequestId, PeerRequestOpened, RequestEvent, RequestId, Session,
};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, PeerRequestStreamEvent, RequestReader, RequestStreamEvent, SendStream, Trace,
    send_message,
};
use xmoq_subscriber::followed::Gap;
use xmoq_viewer::chat_track::{Say, Saying};

use xmoq_subscriber::follow::{self, Asking, Follower, Track, send_request};

pub use xmoq_subscriber::follow::FollowerInput;

type Send = <WtConnection as Connection>::Send;
type Recv = <WtConnection as Connection>::Recv;

/// The most bytes of the stream of the Joining FETCH: 16 lines of
/// [`TEXT_BYTES`] and their JSON, with room.
const FETCH_LIMIT: usize = 64 * 1024;

/// The most an Object of `lines` may be: a line of [`TEXT_BYTES`] and its
/// JSON, with room. The page's chat has the same (`@xmoq/moqt`'s
/// `maxObjectPayloadLength`). **Provisional.**
const LINES_PAYLOAD_MOST: usize = 64 * 1024;

/// How long a stream for a line of `said` may wait to be opened. WHY not
/// waited for longer: a line said late is worth less than the next, as an
/// Object of a live track (the Rust client's `SUBGROUP_OPEN_WAIT`).
const SAID_OPEN_WAIT: Duration = Duration::from_secs(2);

/// What the loop is to do with what came. How a line is shown is the
/// program's: the terminal and the agent's file show it differently.
pub enum ChatOutcome {
    /// A line of the room, each once, in the order of its number.
    Line(Line),
    /// Lines that were not fetched, told before the lines after them.
    Gap(Gap),
    /// A line for the program's log, about the chat on tracks itself.
    Note(String),
}

impl ChatOutcome {
    fn of(out: follow::FollowOutcome<Line>) -> Self {
        match out {
            follow::FollowOutcome::Item(line) => Self::Line(line),
            follow::FollowOutcome::Gap(gap) => Self::Gap(gap),
            follow::FollowOutcome::Note(note) => Self::Note(note),
        }
    }
}

/// The chat on tracks of one session.
pub struct ChatTracks {
    /// `xmoq/chat/<room>/<name>`, `said`.
    said: FullTrackName,
    saying: Saying,
    /// The room's `lines`.
    lines: Follower<Line>,
    /// The request stream of the `PUBLISH_NAMESPACE`, which stays open while
    /// the session lasts.
    _announce: Send,
    /// The half of the `PUBLISH_NAMESPACE`'s stream the relay writes to.
    /// WHY kept, though nothing more is read: letting go of it stops the
    /// stream, and the relay takes the announcement as withdrawn (the
    /// gatherer is told `NAMESPACE_DONE` at once; seen in the first run).
    _announced: RequestReader<Recv>,
    /// The gatherer's SUBSCRIBE to `said`: its stream, request, Track Alias,
    /// and the half of its stream that stays open.
    subscriber: Option<(u64, PeerRequestId, u64, Send)>,
}

impl ChatTracks {
    /// Announces the viewer's namespace and waits for the answer, so that a
    /// relay that refuses it is said at once. `lines` is asked for from the
    /// loop ([`Self::next`]), where the broadcast plays meanwhile. `had` is
    /// the newest line shown on the sessions before, so that what was
    /// missed between them is fetched (`past_to_fetch`), `catch_up` of
    /// them at the most. `asking` carries the `sub` token of the room's
    /// namespace, which `lines` is asked for with.
    ///
    /// # Errors
    ///
    /// A room or name that makes no namespace, a stream the relay does not
    /// allow in time, and a `PUBLISH_NAMESPACE` that is refused or not
    /// answered in time.
    pub async fn begin(
        session: &mut Session,
        asking: Asking,
        publish_token: Option<&[u8]>,
        person: &Person,
        catch_up: u64,
        had: Option<u64>,
    ) -> Result<Self> {
        let tokens: Vec<Parameter> = publish_token
            .map(|token| Parameter::AuthorizationToken(token.to_vec()))
            .into_iter()
            .collect();
        let room = room_namespace(&person.room)?;
        let mine = person_namespace(&person.room, &person.name)?;
        let said = FullTrackName::new(mine.clone(), SAID.as_bytes().to_vec())?;
        let (announce, message) = session.publish_namespace(mine, tokens)?;
        let (announce_send, mut answers) = send_request(
            asking.connection.as_ref(),
            asking.open_wait_ms,
            &message,
            "publish-namespace",
        )
        .await?;
        let answer_wait_ms = asking.answer_wait_ms;
        let answer =
            tokio::time::timeout(Duration::from_millis(answer_wait_ms), answers.next_event())
                .await
                .map_err(|_| {
                    anyhow!("no answer to PUBLISH_NAMESPACE within {answer_wait_ms} ms")
                })?;
        let RequestStreamEvent::Message(answer) = answer? else {
            bail!("the PUBLISH_NAMESPACE was closed without an answer");
        };
        match session.on_request_message(announce, answer)? {
            RequestEvent::PublishNamespaceEstablished => {}
            other => bail!("the PUBLISH_NAMESPACE was answered with {other:?}"),
        }
        let lines = Follower::new(
            Track {
                namespace: room,
                name: LINES,
                // WHY held: the gatherer publishes `lines` of a room once a
                // participant has come, and the viewer that comes first
                // would otherwise be refused before its own announcement
                // reached the gatherer.
                rendezvous_ms: Some(follow::RENDEZVOUS_MS),
                payload_most: LINES_PAYLOAD_MOST,
                fetch_most: FETCH_LIMIT,
                recent: xmoq_viewer::chat_track::RECENT_LINES,
                catch_up,
                note: "CHAT_TRACKS_LINES",
                fetch_note: "CHAT_TRACKS",
                read: |number, payload| read_line(number, payload).ok(),
            },
            asking,
            had,
        );
        Ok(Self {
            said,
            saying: Saying::default(),
            lines,
            _announce: announce_send,
            _announced: answers,
            subscriber: None,
        })
    }

    /// The number of the newest line shown, if any.
    pub fn last_shown(&self) -> Option<u64> {
        self.lines.last_shown()
    }

    /// The Track Alias of `lines`, while it is subscribed to.
    pub fn lines_alias(&self) -> Option<u64> {
        self.lines.track_alias()
    }

    /// The Request ID of the Joining FETCH, while it is open.
    pub fn fetch_request_id(&self) -> Option<u64> {
        self.lines.fetch_request_id()
    }

    /// What comes next.
    pub async fn next(&mut self) -> FollowerInput {
        self.lines.next().await
    }

    /// What came, and what the loop is to do with it.
    ///
    /// # Errors
    ///
    /// As [`Follower::on`]: the session is to be closed.
    pub async fn on(
        &mut self,
        session: &mut Session,
        came: FollowerInput,
    ) -> Result<Vec<ChatOutcome>, RequestFault> {
        Ok(self
            .lines
            .on(session, came)
            .await?
            .into_iter()
            .map(ChatOutcome::of)
            .collect())
    }

    /// Reads the stream of the FETCH on a task of its own; its lines come
    /// back by [`Self::next`].
    pub fn read_fetch(&mut self, stream: u64, header: xmoq_moqt::fetch::FetchHeader) {
        self.lines.read_fetch(stream, header);
    }

    /// The session ended `request` for a malformed track
    /// (`Session::cancel_malformed`): what the loop is to do, where it is
    /// of `lines` ([`Follower::malformed`]); `None` where it is not.
    pub fn malformed(&mut self, request: RequestId) -> Option<Vec<ChatOutcome>> {
        let outs = self.lines.malformed(request)?;
        Some(outs.into_iter().map(ChatOutcome::of).collect())
    }

    /// An Object of `lines`, the Group `number`: the line to show, if it is
    /// shown.
    pub fn live(&mut self, number: u64, payload: &[u8]) -> Option<Line> {
        self.lines.live(number, payload)
    }

    /// A line the viewer says. `false` where it is not sent: too long, or
    /// too many wait.
    pub fn say(&mut self, connection: &Arc<WtConnection>, text: String) -> bool {
        if text.len() > TEXT_BYTES {
            return false;
        }
        match self.saying.say(text) {
            Say::Now(group, text) => {
                self.write(connection, group, &text);
                true
            }
            Say::Held => true,
            Say::Dropped => false,
        }
    }

    /// Writes a line on `said` as its own Group, on a task: one stream,
    /// Object 0, a FIN.
    fn write(&self, connection: &Arc<WtConnection>, group: u64, text: &str) {
        let Some((_, _, track_alias, _)) = self.subscriber.as_ref() else {
            return;
        };
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias: *track_alias,
            group_id: group,
            subgroup_id: SubgroupId::Explicit(0),
            publisher_priority: Some(128),
            properties: false,
            // The line is the Group's one Object.
            end_of_group: true,
            first_object: true,
        });
        let sent_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            });
        let object = SubgroupObject {
            object_id: 0,
            properties: vec![],
            status: ObjectStatus::Normal,
            payload: SentLine {
                text: text.to_string(),
                sent_at,
            }
            .payload(),
        };
        if encoder.object(&object).is_err() {
            return;
        }
        let bytes = encoder.take_bytes();
        let connection = Arc::clone(connection);
        tokio::spawn(async move {
            let Ok(Ok(mut stream)) =
                tokio::time::timeout(SAID_OPEN_WAIT, connection.open_uni()).await
            else {
                return;
            };
            if stream.write_all(&bytes).await.is_ok() {
                stream.finish();
            }
        });
    }

    /// A request stream the peer opened. The gatherer's SUBSCRIBE to `said`
    /// is taken, and what is said waits no more; anything else is handed
    /// back, for the viewer to refuse.
    ///
    /// # Errors
    ///
    /// What the session takes as an error of the peer, and a `SUBSCRIBE_OK`
    /// that cannot be sent.
    pub async fn on_peer_request(
        &mut self,
        session: &mut Session,
        connection: &Arc<WtConnection>,
        event: PeerRequestStreamEvent<Send>,
    ) -> Result<Option<PeerRequestStreamEvent<Send>>> {
        match event {
            PeerRequestStreamEvent::Opened {
                stream,
                mut send,
                first: ControlMessage::Subscribe(subscribe),
            } if subscribe.full_track_name == self.said && self.subscriber.is_none() => {
                let opened =
                    session.on_peer_request_opened(ControlMessage::Subscribe(subscribe))?;
                let request = match opened {
                    PeerRequestOpened::Request(request, PeerRequest::Subscribe(_)) => request,
                    PeerRequestOpened::Refused { response, .. } => {
                        let _ =
                            send_message(&mut send, &response, &Trace::default(), "request").await;
                        send.finish();
                        return Ok(None);
                    }
                    PeerRequestOpened::Request(..) => {
                        bail!("a SUBSCRIBE was read as another request")
                    }
                };
                let (track_alias, ok) = session.accept_subscribe(request, vec![], vec![])?;
                send_message(&mut send, &ok, &Trace::default(), "said").await?;
                self.subscriber = Some((stream, request, track_alias, send));
                for (group, text) in self.saying.subscribed() {
                    self.write(connection, group, &text);
                }
                Ok(None)
            }
            PeerRequestStreamEvent::Message { stream, message }
                if self
                    .subscriber
                    .as_ref()
                    .is_some_and(|(said, ..)| *said == stream) =>
            {
                if let Some((_, request, ..)) = &self.subscriber {
                    session.on_peer_request_message(*request, message)?;
                }
                Ok(None)
            }
            PeerRequestStreamEvent::Closed { stream, .. }
                if self
                    .subscriber
                    .as_ref()
                    .is_some_and(|(said, ..)| *said == stream) =>
            {
                if let Some((_, request, ..)) = self.subscriber.take() {
                    session.on_peer_request_closed(request);
                }
                self.saying.unsubscribed();
                Ok(None)
            }
            other => Ok(Some(other)),
        }
    }
}

/// The rows of `fixtures/followed-track/limits.tsv` whose Rust value is
/// here, against the table: the payload of `lines`.
#[cfg(test)]
#[test]
fn the_payload_of_lines_is_as_the_shared_table_says() {
    let path = format!(
        "{}/../../fixtures/followed-track/limits.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let table = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    assert!(table.ends_with('\n') && !table.ends_with("\n\n"), "{path}");
    let ours = |name: &str| -> Option<Option<u64>> {
        let count = |value: usize| u64::try_from(value).ok();
        match name {
            "lines-payload-bytes" => Some(count(LINES_PAYLOAD_MOST)),
            _ => None,
        }
    };
    let mut read = Vec::new();
    for row in table.lines() {
        let [name, rust, _] = row.split('\t').collect::<Vec<_>>()[..] else {
            panic!("a row of three columns: {row:?}");
        };
        let Some(value) = ours(name) else { continue };
        let expected = (rust != "-").then(|| rust.parse::<u64>().expect("a number"));
        assert_eq!(value, expected, "{name}");
        read.push(name);
    }
    // As many as are here: a table that lost a row would pass.
    assert_eq!(read.len(), 1, "{read:?}");
}
