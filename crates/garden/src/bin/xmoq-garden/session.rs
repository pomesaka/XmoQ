//! One `MoQ` Transport session of the garden with the relay: what it asks for,
//! what it publishes, and the FETCH requests it answers.
//!
//! The garden is built as the chat's gatherer is
//! (`crates/chat/src/bin/xmoq-chat/session.rs`, where the reasons are): one
//! `SUBSCRIBE_NAMESPACE` for the prefix, a SUBSCRIBE with the Largest
//! Object filter for the `acted` of each actor, and a PUBLISH, not a
//! `PUBLISH_NAMESPACE`, for the `garden` of each room from its first actor
//! until the room is forgotten, which ends with `PUBLISH_DONE`
//! `TRACK_ENDED` then (§10.11). WHY PUBLISH and not `PUBLISH_NAMESPACE` of
//! the room: the namespace of a room is a prefix of its actors', and a
//! garden that announced it would be asked by the relay for every `acted`
//! of the room (§9.5, "to all matching publishers").

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use tokio::sync::{mpsc, watch};
use xmoq_again::next_session::{Pause, new_url};
use xmoq_garden::args::AskAgain;
use xmoq_garden::rooms::{Joined, Left, Rooms, Written};
use xmoq_garden::tracks;
use xmoq_moqt::data::ObjectStatus;
use xmoq_moqt::early::{Arrived, EarlyObjects, held_bytes};
use xmoq_moqt::immutable::TrackRefusal;
use xmoq_moqt::message::{FetchRange, publish_done_status, request_error_code};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::{GroupOrder, Parameter, SubscriptionFilter};
use xmoq_moqt::session::{
    ControlEvent, MalformedRequest, RequestEvent, RequestId, Role, Session, SubscribedAlias,
    Transport,
};
use xmoq_moqt::{RequestFault, stream_reset_code};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, PeerRequestStreamEvent, RequestClose, RequestReadError, StreamConfig, StreamEnd,
    StreamStops, UniEvent, client, read_request_streams, read_uni_streams,
};
use xmoq_pet_tracks::names::{Actor, GardenTrack, PREFIX};
use xmoq_pet_tracks::payloads::read_action;
use xmoq_publisher::answers::{self, Bounds, MAX_ANSWERED_AT_ONCE, Note};
use xmoq_publisher::fetch::pieces_of;
use xmoq_publisher::own::OwnRequests;
use xmoq_publisher::publication::{PublishStage, RequestStreamEnd, StreamEndDoes, stream_end};
use xmoq_publisher::publishing::{self, Publish, Publishing, Sent};
use xmoq_publisher::requests::{AskedFetch, FetchAnswer, PeerRequests};
use xmoq_publisher::streams::{COMMAND_QUEUE, Command, Heard};
use xmoq_publisher::{shown, sleep_until};
use xmoq_room_token::{Role as TokenRole, RoomKey};
use xmoq_subscriber::kept::{Becomes, Happened, Tries, decide};
use xmoq_subscriber::namespaces::KeptNamespace;

type Send = <WtConnection as Connection>::Send;

/// How long a request may wait for the relay to allow its stream, as the
/// gatherer waits. Provisional.
const OPEN_WAIT: Duration = Duration::from_secs(10);

/// Events of the relay's unidirectional streams not taken yet; the readers
/// wait when it is full, and flow control holds the relay back.
/// Provisional.
const UNI_QUEUE: usize = 64;

/// Requests the relay opened that are not taken yet; as above.
const PEER_REQUEST_QUEUE: usize = 16;

/// What the tasks of the garden's request streams said and the loop did not
/// take yet; they wait when it is full. Provisional.
const HEARD_QUEUE: usize = 256;

/// The most bytes the garden holds of one stream of `acted` before it has
/// decoded the Object on it; a stream with more is stopped, and its action
/// is lost. WHY 4 KiB: an action is under 300 bytes (a talk of 120 bytes of
/// text, escaped). **Provisional.**
const PAYLOAD_MOST: usize = 4 * 1024;

/// The gardens of a room that wait for its `PUBLISH_OK`, the oldest dropped
/// first. WHY 2: each says the whole garden, which the next says anew.
/// **Provisional.**
const GARDENS_WAITING_FOR_A_ROOM: usize = 2;

/// Subgroup streams of gardens being written at once. Past it a garden is not
/// sent live; the next says it anew, at the latest [`GARDEN_AGAIN_MS`] after.
/// **Provisional.**
///
/// [`GARDEN_AGAIN_MS`]: xmoq_garden::rooms::GARDEN_AGAIN_MS
const GARDENS_BEING_SENT: usize = 256;

/// The most bytes of payload the answers to FETCH requests being written
/// hold together. WHY 1 MiB: an answer is one garden, of under 16 KiB where
/// the names of the actors are short (16 talks of 120 bytes, escaped, and 8
/// things), and [`MAX_ANSWERED_AT_ONCE`] are answered at once. **Provisional.**
const MAX_BYTES_HELD: usize = 1024 * 1024;

/// An Object of a room's `garden` that waits for its `PUBLISH_OK`: its Group
/// ID and its payload.
type Waiting = (u64, Vec<u8>);

/// How a session ended without an error.
#[derive(Debug)]
pub enum SessionEnd {
    /// The garden was told to stop.
    Stopped,
    /// The relay sent GOAWAY on the control stream (§10.4), with the URL
    /// to go to, or none for the same.
    Moved(Option<String>),
}

/// The subscription to the `acted` of a followed actor.
struct Followed {
    /// The token of its SUBSCRIBE, while it has one.
    token: Option<u64>,
    /// When it is asked again after it is over.
    tries: Tries,
}

/// What a request of the garden is for.
#[derive(Debug, Clone)]
enum Purpose {
    /// The `SUBSCRIBE_NAMESPACE` for `xmoq/pet`.
    Watch,
    /// The SUBSCRIBE for the `acted` of an actor.
    Actor(Actor),
    /// The PUBLISH of the `garden` of a room.
    Room(String),
}

/// Everything of one session.
struct Tending<'a> {
    connection: &'a Arc<WtConnection>,
    rooms: &'a mut Rooms,
    session: Session,
    stops: StreamStops,
    own: OwnRequests<Purpose>,
    /// The `garden` of each room that is kept, with the gardens
    /// that wait for its `PUBLISH_OK`.
    publishing: Publishing<String, Waiting>,
    /// Each actor followed in this session.
    actors: HashMap<Actor, Followed>,
    /// The actors whose `acted` is to be asked for again, and when.
    asking_again: HashMap<Actor, Instant>,
    ask_again: AskAgain,
    /// Objects of `acted` read before the `SUBSCRIBE_OK` that gives their
    /// Track Alias (`xmoq_moqt::early`).
    early: EarlyObjects<(u64, Vec<u8>)>,
    /// The FETCH requests of the relay being answered, and those refused.
    requests: PeerRequests,
    watch: KeptNamespace,
    /// `--auth-key-hex`.
    key: Option<&'a RoomKey>,
}

/// The garden's clock since the epoch, in milliseconds: the times of `garden`.
fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| u64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

/// What the session says the peer did wrong, as an error that carries its
/// code (`xmoq_moqt_io::client::termination_code`).
fn fault(fault: RequestFault) -> anyhow::Error {
    match fault {
        RequestFault::Peer(error) => error.into(),
        RequestFault::Usage(error) => anyhow!("the garden used its session wrongly: {error}"),
    }
}

/// Runs one session on `connection` until it ends.
#[expect(
    clippy::too_many_lines,
    reason = "the readers, the garden and the one loop of the session, which share their queues"
)]
pub async fn tend(
    connection: &Arc<WtConnection>,
    kept: &mut client::UntilClosed<WtConnection>,
    rooms: &mut Rooms,
    key: Option<&RoomKey>,
    ask_again: AskAgain,
    stop: &mut watch::Receiver<bool>,
) -> Result<SessionEnd> {
    let stops = StreamStops::default();
    let (uni_queue, mut uni) = mpsc::channel(UNI_QUEUE);
    let stream_config = StreamConfig {
        subgroup_buffer_limit: PAYLOAD_MOST,
        ..StreamConfig::default()
    };
    let uni_readers = read_uni_streams(
        Arc::clone(connection),
        stream_config.clone(),
        stops.clone(),
        uni_queue,
        |event| event,
    );
    let (peer_queue, mut peer) = mpsc::channel(PEER_REQUEST_QUEUE);
    let request_readers = read_request_streams(
        Arc::clone(connection),
        stream_config,
        stops.clone(),
        peer_queue,
        |event| event,
    );
    kept.readers.extend([uni_readers, request_readers]);
    let (heard, mut heard_by_loop) = mpsc::channel(HEARD_QUEUE);
    let (drained, mut drained_rooms) = mpsc::channel(PEER_REQUEST_QUEUE);
    let mut tending = Tending {
        connection,
        rooms,
        session: Session::new(Role::Client, Transport::WebTransport),
        stops,
        own: OwnRequests::new(Arc::clone(connection), heard, OPEN_WAIT),
        publishing: Publishing::new(drained, GARDENS_BEING_SENT),
        actors: HashMap::new(),
        asking_again: HashMap::new(),
        ask_again,
        early: EarlyObjects::default(),
        requests: PeerRequests::new(Bounds {
            at_once: MAX_ANSWERED_AT_ONCE,
            bytes: MAX_BYTES_HELD,
            say_queue: COMMAND_QUEUE,
        }),
        watch: KeptNamespace::new(tracks::prefix()?, Pause::SERVING),
        key,
    };

    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    // The relay looks at no namespace or role of the token of a SETUP.
    let token = tending.token(TokenRole::Sub);
    let options = client::setup_options(token.as_deref(), implementation)?;
    let set_up = client::setup(
        connection.as_ref(),
        &mut tending.session,
        &mut uni,
        &mut kept.control,
        options,
        client::SETUP_WAIT,
    )
    .await?;
    println!("SETUP_OK");
    // What came before the SETUP (§3.3) is taken first.
    for event in set_up.early {
        if let Some(ended) = tending.on_uni(event)? {
            return Ok(ended);
        }
    }
    // Who is in the rooms is what this session is told; the gardens stay.
    tending.rooms.leave_everybody(wall_ms());
    tending.watch()?;

    let ended = loop {
        // WHY taken before the select: its arms borrow parts of the state
        // mutably.
        let (next_watch, next_asked, next_room) = (
            tending.watch.again_at(),
            tending.asking_again.values().min().copied(),
            tending.next_room(),
        );
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break SessionEnd::Stopped;
                }
            }
            event = uni.recv() => {
                let event = event.ok_or_else(|| anyhow!("the connection is gone"))?;
                if let Some(ended) = tending.on_uni(event)? {
                    break ended;
                }
            }
            event = peer.recv() => {
                let event = event.ok_or_else(|| anyhow!("the connection is gone"))?;
                tending.on_peer_request(event)?;
            }
            Some((token, heard)) = heard_by_loop.recv() => {
                if let Some(ended) = tending.on_heard(token, heard)? {
                    break ended;
                }
            }
            note = tending.requests.next_note() => tending.fetch_noted(note),
            Some(token) = drained_rooms.recv() => tending.publish_done(token)?,
            () = sleep_until(next_watch) => {
                if tending.watch.due(Instant::now()) {
                    tending.watch()?;
                }
            }
            () = sleep_until(next_asked) => tending.ask_again()?,
            () = sleep_until(next_room) => tending.go_on()?,
            () = tending.publishing.sent() => {}
        }
    };
    Ok(ended)
}

impl Tending<'_> {
    /// A token of the garden's own for `role` in every namespace of the garden
    /// (`xmoq/pet/*`) from now, as an AUTHORIZATION TOKEN; `None` without
    /// `--auth-key-hex`.
    ///
    /// WHY made anew for each request, and for `xmoq/pet/*`: as the chat's
    /// gatherer makes its own (`Gathering::token`); the relay takes a
    /// `SUBSCRIBE_NAMESPACE` with a token whose last field is `*` alone
    /// (`crates/relay/src/auth.rs`), and the garden follows every room.
    fn token(&self, role: TokenRole) -> Option<Vec<u8>> {
        let key = self.key?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let namespace = format!("{}/*", PREFIX.join("/"));
        Some(key.authorization_token(&namespace, role, now))
    }

    /// [`Self::token`] as the parameter of a request.
    fn token_parameter(&self, role: TokenRole) -> Option<Parameter> {
        self.token(role).map(Parameter::AuthorizationToken)
    }

    /// Asks for the namespaces under `xmoq/pet`.
    fn watch(&mut self) -> Result<()> {
        self.watch.asked(Instant::now());
        let prefix = self.watch.prefix().clone();
        let (request, message) = self.session.subscribe_namespace(
            prefix,
            self.token_parameter(TokenRole::Sub).into_iter().collect(),
        )?;
        self.own.open(Purpose::Watch, request, message, &[]);
        Ok(())
    }

    /// Says that the `SUBSCRIBE_NAMESPACE` is asked for again `wait_ms` from
    /// now.
    fn watch_ended(&self, why: &str, wait_ms: u64) {
        println!(
            "WATCH_ENDED prefix={} why={why:?} again_in_ms={wait_ms}",
            shown(self.watch.prefix())
        );
    }

    /// Tells the task of a request stream something; where its queue is
    /// full, the request is cancelled with `EXCESSIVE_LOAD` (§3.3.3), as the
    /// gatherer does (why, there).
    fn tell(&mut self, token: u64, command: Command) {
        if self.own.tell(token, command).is_err()
            && let Some(purpose) =
                self.let_go(token, Command::Cancel(stream_reset_code::EXCESSIVE_LOAD))
        {
            self.request_over(token, &purpose, Happened::Ended, "its queue was full");
        }
    }

    /// Ends a request in the session and on its stream, and forgets it.
    fn let_go(&mut self, token: u64, command: Command) -> Option<Purpose> {
        self.own.let_go(&mut self.session, token, command)
    }

    /// Publishes the `garden` of a room, unless it is published or waits to
    /// be published again.
    fn publish(&mut self, room: &str) -> Result<()> {
        let key = room.to_string();
        self.publishing
            .track(key.clone(), GARDENS_WAITING_FOR_A_ROOM);
        let largest = self
            .rooms
            .get(room)
            .and_then(xmoq_garden::rooms::Room::latest_group);
        let parameters = publishing::parameters(largest, self.token_parameter(TokenRole::Pub));
        let published = self.publishing.publish(
            &key,
            tracks::garden(room)?,
            &parameters,
            &mut self.session,
            &mut self.own,
            Purpose::Room(key.clone()),
        );
        if let Publish::Refused(_) = published {
            println!("ROOM_NOT_PUBLISHED room={room:?} why=\"the PUBLISH before is ending\"");
        }
        Ok(())
    }

    /// Ends the PUBLISH of a room's `garden` with `status`; its
    /// `PUBLISH_DONE` goes once its streams are closed ([`Self::publish_done`]).
    /// `forget`: the room is forgotten, and its publication goes.
    fn end_publish(&mut self, room: &str, status: u64, forget: bool) {
        let key = room.to_string();
        self.publishing.end(&key, status);
        if forget {
            self.publishing.forget(&key);
        }
    }

    /// Every stream of an ending PUBLISH is closed: its `PUBLISH_DONE`
    /// (§10.11) and a FIN. Where the room is still kept, it is published
    /// anew.
    fn publish_done(&mut self, token: u64) -> Result<()> {
        let Some(done) = self.publishing.done(token, &mut self.session, &self.own)? else {
            return Ok(());
        };
        let room = done.key;
        if let Some(message) = done.message {
            self.tell(token, Command::Say(message));
            self.let_go(token, Command::Finish);
            println!(
                "ROOM_DONE room={room:?} status={:#x} streams={}",
                done.status, done.streams
            );
        }
        if self.rooms.get(&room).is_some() {
            if let Some(entry) = self.publishing.get_mut(&room) {
                entry.may_publish_again();
            }
            self.publish(&room)?;
        }
        Ok(())
    }

    /// The PUBLISH of a room is over; the next garden publishes it again.
    fn unpublish(&mut self, room: &str, token: u64) {
        if let Some(entry) = self.publishing.get_mut(&room.to_string())
            && entry.unpublished(token, true, Instant::now())
        {
            println!("ROOM_UNPUBLISHED room={room:?}");
        }
    }

    /// Subscribes to the `acted` of an actor, with the Largest Object
    /// filter (§5.1.2), as the gatherer subscribes to `said` (why, there):
    /// an action the relay still has of before is not taken again.
    fn subscribe(&mut self, actor: &Actor) -> Result<()> {
        let mut parameters = vec![
            Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
            Parameter::Forward(true),
            Parameter::SubscriberPriority(128),
            Parameter::GroupOrder(GroupOrder::Ascending),
        ];
        parameters.extend(self.token_parameter(TokenRole::Sub));
        self.asking_again.remove(actor);
        let Ok((request, message)) = self
            .session
            .subscribe(tracks::acted(actor)?, parameters.clone())
        else {
            println!(
                "NOT_SUBSCRIBED room={:?} name={:?} why=\"a subscription is there already\"",
                actor.room, actor.name
            );
            return Ok(());
        };
        let token = self
            .own
            .open(Purpose::Actor(actor.clone()), request, message, &parameters);
        if let Some(followed) = self.actors.get_mut(actor) {
            followed.token = Some(token);
        }
        Ok(())
    }

    /// The `acted` of a actor that stays announced is over: it is
    /// asked for again after a pause, or not until its NAMESPACE comes again
    /// (`kept::decide` of `xmoq-subscriber`).
    fn acted_over(&mut self, actor: &Actor, end: Happened, why: &str) {
        println!(
            "NOT_SUBSCRIBED room={:?} name={:?} why={why:?}",
            actor.room, actor.name
        );
        let Some(followed) = self.actors.get_mut(actor) else {
            return;
        };
        followed.token = None;
        match decide(end, self.ask_again.keeping(), &mut followed.tries) {
            Becomes::Again(pause) => {
                println!(
                    "ASKING_AGAIN room={:?} name={:?} in_ms={pause}",
                    actor.room, actor.name
                );
                self.asking_again
                    .insert(actor.clone(), Instant::now() + Duration::from_millis(pause));
            }
            Becomes::GivenUp | Becomes::Subscribed => println!(
                "NOT_ASKING_AGAIN room={:?} name={:?}",
                actor.room, actor.name
            ),
        }
    }

    /// The Objects held for an alias the session knows now are taken, and
    /// the streams of those no SUBSCRIBE can give any more are stopped.
    fn settle_early(&mut self) -> Result<()> {
        if self.early.is_empty() {
            return Ok(());
        }
        let settled = self.early.settle(&self.session);
        for (stream, code) in settled.stopped {
            self.stops.stop(stream, code);
        }
        for (track_alias, payload) in settled.released {
            self.on_object(track_alias, &payload)?;
        }
        Ok(())
    }

    /// Asks again for every `acted` whose time has come.
    fn ask_again(&mut self) -> Result<()> {
        let now = Instant::now();
        let due: Vec<Actor> = self
            .asking_again
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(actor, _)| actor.clone())
            .collect();
        for actor in due {
            self.asking_again.remove(&actor);
            if self
                .actors
                .get(&actor)
                .is_some_and(|followed| followed.token.is_none())
            {
                self.subscribe(&actor)?;
            }
        }
        Ok(())
    }

    /// A NAMESPACE under `xmoq/pet`.
    fn on_namespace(&mut self, namespace: &TrackNamespace) -> Result<()> {
        let Some(actor) = tracks::actor_of(namespace) else {
            println!("NAMESPACE_IGNORED namespace={}", shown(namespace));
            return Ok(());
        };
        let joined = self.rooms.join(&actor);
        if matches!(joined, Joined::TooManyRooms | Joined::TooManyActors) {
            println!(
                "ACTOR_IGNORED room={:?} name={:?} why={}",
                actor.room,
                actor.name,
                joined.word()
            );
            return Ok(());
        }
        // A NAMESPACE that comes again counts the times anew.
        let tries = self.ask_again.tries();
        let followed = self
            .actors
            .entry(actor.clone())
            .or_insert_with(|| Followed {
                token: None,
                tries: tries.clone(),
            });
        followed.tries = tries;
        let subscribed = followed.token.is_some();
        if let Joined::Now { .. } = joined {
            println!("ACTOR room={:?} name={:?}", actor.room, actor.name);
            // One who comes gets a garden at once; the first actor of a room
            // has it published.
            self.send_garden(&actor.room, false)?;
        }
        if !subscribed {
            self.subscribe(&actor)?;
        }
        Ok(())
    }

    /// A `NAMESPACE_DONE` under `xmoq/pet`: the actor is gone. Its room stays
    /// published until it is forgotten ([`Self::go_on`]).
    fn on_namespace_done(&mut self, namespace: &TrackNamespace) {
        let Some(actor) = tracks::actor_of(namespace) else {
            return;
        };
        self.asking_again.remove(&actor);
        if let Some(Followed {
            token: Some(token), ..
        }) = self.actors.remove(&actor)
        {
            self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
        }
        if self.rooms.leave(&actor, wall_ms()) != Left::NotThere {
            println!("ACTOR_GONE room={:?} name={:?}", actor.room, actor.name);
        }
    }

    /// An Object of `acted`: an action, which changes the room's garden by
    /// its rules (`xmoq_garden::rules`), or is dropped.
    fn on_acted(&mut self, actor: &Actor, payload: &[u8]) -> Result<()> {
        let dropped = |why: &str| {
            println!(
                "ACTED_DROPPED room={:?} name={:?} why={why}",
                actor.room, actor.name
            );
        };
        if !self.rooms.follows(actor) {
            dropped("not-followed");
            return Ok(());
        }
        let action = match read_action(payload) {
            Ok(action) => action,
            Err(error) => {
                dropped(error.word());
                return Ok(());
            }
        };
        let Some(room) = self.rooms.get_mut(&actor.room) else {
            dropped("not-followed");
            return Ok(());
        };
        if let Err(why) = room.garden.act(&actor.name, &action.kind, wall_ms()) {
            dropped(why.word());
            return Ok(());
        }
        println!(
            "ACTED room={:?} name={:?} kind={}",
            actor.room,
            actor.name,
            action.kind.word()
        );
        self.send_garden(&actor.room, true)
    }

    /// An Object of a track the garden subscribed to under `track_alias`.
    fn on_object(&mut self, track_alias: u64, payload: &[u8]) -> Result<()> {
        let track = match self.session.subscribed_alias(track_alias) {
            SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => {
                tracks::track_of(track)
            }
            SubscribedAlias::Unknown | SubscribedAlias::Retired => None,
        };
        // The garden subscribes to `acted` alone.
        if let Some(GardenTrack::Acted { room, name }) = track {
            self.on_acted(&Actor { room, name }, payload)?;
        }
        Ok(())
    }

    /// Sends the garden of a room as it is now, as a Group of its own, or has
    /// it wait for the room's `PUBLISH_OK`. `changed`: an action changed it,
    /// and it is said in the log; a garden sent again is not.
    fn send_garden(&mut self, room: &str, changed: bool) -> Result<()> {
        let Some(entry) = self.rooms.get_mut(room) else {
            return Ok(());
        };
        let Written {
            group: group_id,
            payload,
            things,
            events,
        } = entry.write(wall_ms());
        if changed {
            println!("GARDEN room={room:?} things={things} events={events}");
        }
        let key = room.to_string();
        if !self.publishing.live(&key) {
            self.publish(room)?;
            self.publishing.wait(&key, (group_id, payload));
            return Ok(());
        }
        let shown = key.clone();
        let failed = move |why: String| {
            println!("GARDEN_NOT_SENT room={shown:?} group={group_id} why={why:?}");
        };
        let sent = self
            .publishing
            .send(&key, self.connection, (group_id, payload), failed);
        if sent == Sent::TooMany {
            println!("GARDEN_NOT_SENT room={room:?} group={group_id} why=too-many-at-once");
        }
        Ok(())
    }

    /// When a room next has something to do.
    fn next_room(&self) -> Option<Instant> {
        let at = self.rooms.next_at()?;
        let wait = at.saturating_sub(wall_ms());
        Some(Instant::now() + Duration::from_millis(wait))
    }

    /// What is due of every room: its garden published again, or the room
    /// forgotten and its PUBLISH ended.
    fn go_on(&mut self) -> Result<()> {
        let now = wall_ms();
        for name in self.rooms.names() {
            if self
                .rooms
                .get(&name)
                .and_then(xmoq_garden::rooms::Room::again_at)
                .is_some_and(|at| at <= now)
            {
                self.send_garden(&name, false)?;
            }
        }
        for gone in self.rooms.forget(now) {
            println!("ROOM_FORGOTTEN room={gone:?}");
            self.end_publish(&gone, publish_done_status::TRACK_ENDED, true);
        }
        Ok(())
    }

    /// An event of the relay's unidirectional streams.
    fn on_uni(&mut self, event: UniEvent) -> Result<Option<SessionEnd>> {
        self.settle_early()?;
        match event {
            UniEvent::Control(message) => match self.session.on_control_message(message)? {
                ControlEvent::GoAway(go_away) => {
                    println!("GOAWAY");
                    return Ok(Some(SessionEnd::Moved(new_url(&go_away.new_session_uri))));
                }
                ControlEvent::Setup(_) => bail!("the session took a second SETUP"),
            },
            UniEvent::Object {
                stream,
                header,
                object,
            } => {
                if object.status != ObjectStatus::Normal {
                    return Ok(None);
                }
                let (alias, bytes) = (header.track_alias, held_bytes(&object));
                let item = (alias, object.payload);
                let payload = match self.early.arrive(&self.session, stream, alias, bytes, item) {
                    Arrived::Deliver((_, payload)) => payload,
                    Arrived::Held => return Ok(None),
                    Arrived::Stop { stream, code } => {
                        self.stops.stop(stream, code);
                        return Ok(None);
                    }
                };
                self.on_object(alias, &payload)?;
            }
            UniEvent::SubgroupClosed {
                stream,
                header,
                end,
            } => {
                self.early
                    .stream_closed(stream, !matches!(end, StreamEnd::Fin));
                self.session.on_data_stream_closed(header.track_alias);
            }
            UniEvent::TrackMalformed { header, error, .. } => {
                println!(
                    "TRACK_MALFORMED track_alias={} error={error}",
                    header.track_alias
                );
                for cancelled in self.session.cancel_malformed(header.track_alias) {
                    match cancelled {
                        MalformedRequest::Subscribe(request) | MalformedRequest::Fetch(request) => {
                            self.malformed(request);
                        }
                        // It takes no PUBLISH of the relay.
                        MalformedRequest::Publish(_) => {}
                    }
                }
            }
            // The garden sends no FETCH: the stream is stopped.
            UniEvent::FetchOpened { stream, header } => {
                self.session.on_fetch_stream(header.request_id)?;
                drop(self.stops.take_fetch_body(stream));
            }
            // Not asked for (`StreamConfig::report_objects_begun`).
            UniEvent::SubgroupOpened { .. } | UniEvent::ObjectBegun { .. } => {}
            UniEvent::Failed(error) => return Err(error.into()),
            UniEvent::ReaderPanicked => return Err(client::ReaderPanicked.into()),
        }
        Ok(None)
    }

    /// What the task of one of the garden's request streams heard.
    fn on_heard(&mut self, token: u64, heard: Heard) -> Result<Option<SessionEnd>> {
        let Some(own) = self.own.get(token) else {
            return Ok(None);
        };
        let (purpose, request) = (own.purpose.clone(), own.request);
        match heard {
            Heard::Opened => Ok(None),
            Heard::NotOpened(why) => {
                self.session.cancel(request);
                self.own.forget(token);
                self.request_over(
                    token,
                    &purpose,
                    Happened::NotSent,
                    &format!("not sent: {why}"),
                );
                Ok(None)
            }
            Heard::NotAnswered => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, &purpose, Happened::NotSent, "not answered in time");
                Ok(None)
            }
            Heard::Message(message) => {
                let event = self
                    .session
                    .on_request_message(request, message)
                    .map_err(fault)?;
                self.on_answer(token, &purpose, event)
            }
            // What the end of the stream of a PUBLISH does to it is
            // `stream_end` of `xmoq-publisher`.
            Heard::Closed(_) | Heard::Stopped(_) if matches!(purpose, Purpose::Room(..)) => {
                let end = match &heard {
                    Heard::Closed(RequestClose::Fin) => RequestStreamEnd::Fin,
                    Heard::Stopped(_) => RequestStreamEnd::StopSending,
                    _ => RequestStreamEnd::Reset,
                };
                let stage = (match &purpose {
                    Purpose::Room(room) => self
                        .publishing
                        .get(room)
                        .and_then(|entry| entry.stage(token)),
                    _ => None,
                })
                .unwrap_or(PublishStage::Established);
                match stream_end(stage, end) {
                    StreamEndDoes::Ends => {
                        self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                        let why = match &heard {
                            Heard::Stopped(code) => format!("stopped ({code:?})"),
                            Heard::Closed(how) => format!("its stream ended ({how})"),
                            _ => String::new(),
                        };
                        self.request_over(token, &purpose, Happened::Ended, &why);
                    }
                    StreamEndDoes::Nothing | StreamEndDoes::PeerEnd => {
                        if matches!(heard, Heard::Closed(RequestClose::Fin)) {
                            println!("PUBLISH_PEER_FINISHED_ITS_DIRECTION");
                        }
                    }
                }
                Ok(None)
            }
            Heard::Closed(how) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                let why = format!("its stream ended ({how})");
                self.request_over(token, &purpose, Happened::Ended, &why);
                Ok(None)
            }
            Heard::Stopped(code) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                let why = format!("stopped ({code:?})");
                self.request_over(token, &purpose, Happened::Ended, &why);
                Ok(None)
            }
            Heard::Failed(RequestReadError::Peer(error)) => Err(error.into()),
            Heard::Failed(RequestReadError::Lost) => bail!("the connection is gone"),
        }
    }

    /// The relay sent GOAWAY on the stream of a request of the garden: the
    /// request alone is asked again, on this session (§10.4), as the
    /// gatherer does.
    ///
    /// FIXME: a New Session URI names another session for the request, and
    /// the request is asked again on this one (§10.4: "on a session at the
    /// specified URI"); the garden keeps one session.
    fn request_moved(&mut self, token: u64, purpose: &Purpose) {
        match purpose {
            Purpose::Room(room) => {
                if self.publishing.is_publish(room, token) {
                    let room = room.clone();
                    self.end_publish(&room, publish_done_status::GOING_AWAY, false);
                }
            }
            purpose => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, purpose, Happened::Ended, "GOAWAY");
            }
        }
    }

    /// A request of the garden is over, and was let go of. The
    /// `SUBSCRIBE_NAMESPACE` is asked for again alone, after a wait that
    /// grows, and the session goes on, as the gatherer's (why, there).
    fn request_over(&mut self, token: u64, purpose: &Purpose, end: Happened, why: &str) {
        match purpose {
            Purpose::Watch => {
                let wait = self.watch.ended(Instant::now());
                self.watch_ended(why, wait);
            }
            Purpose::Actor(actor) => self.acted_over(actor, end, why),
            Purpose::Room(room) => {
                let room = room.clone();
                println!("ROOM_NOT_PUBLISHED room={room:?} why={why:?}");
                self.unpublish(&room, token);
                self.publishing.over(token);
            }
        }
    }

    /// The track of `request` is malformed, and the session ended the
    /// request: §2.4.2, "it MUST cancel any corresponding subscription or
    /// fetches for that Track". Its stream is reset with `MALFORMED_TRACK`.
    fn malformed(&mut self, request: RequestId) {
        let Some(token) = self.own.token_of(request) else {
            return;
        };
        let Some(purpose) = self.let_go(token, Command::Cancel(stream_reset_code::MALFORMED_TRACK))
        else {
            return;
        };
        match &purpose {
            Purpose::Actor(actor) => {
                self.acted_over(actor, Happened::Malformed, "the track is malformed");
            }
            purpose => self.request_over(token, purpose, Happened::Ended, "the track is malformed"),
        }
    }

    /// The `PUBLISH_OK` of a room's `garden`: what waited for it is sent.
    fn room_published(&mut self, token: u64, room: &str, forward: bool) {
        let key = room.to_string();
        let waiting = self
            .publishing
            .get_mut(&key)
            .map(|entry| entry.established(token, forward))
            .unwrap_or_default();
        println!("ROOM_PUBLISHED room={room:?} forward={forward}");
        for (group_id, payload) in waiting {
            let shown = key.clone();
            let failed = move |why: String| {
                println!("GARDEN_NOT_SENT room={shown:?} group={group_id} why={why:?}");
            };
            if self
                .publishing
                .send(&key, self.connection, (group_id, payload), failed)
                == Sent::TooMany
            {
                println!("GARDEN_NOT_SENT room={room:?} group={group_id} why=too-many-at-once");
            }
        }
    }

    /// The meaning of a message on a request stream of the garden.
    fn on_answer(
        &mut self,
        token: u64,
        purpose: &Purpose,
        event: RequestEvent,
    ) -> Result<Option<SessionEnd>> {
        match (purpose, event) {
            (_, RequestEvent::GoAway(_)) => {
                println!("GOAWAY on a request stream");
                self.request_moved(token, purpose);
            }
            (Purpose::Watch, RequestEvent::NamespacesSubscribed) => {
                self.watch.subscribed();
                println!("WATCHING prefix={}", shown(self.watch.prefix()));
            }
            (Purpose::Watch, RequestEvent::Namespace(namespace)) => {
                self.on_namespace(&namespace)?;
            }
            (Purpose::Watch, RequestEvent::NamespaceDone(namespace)) => {
                self.on_namespace_done(&namespace);
            }
            (Purpose::Actor(actor), RequestEvent::SubscribeEstablished { .. }) => {
                println!("SUBSCRIBED room={:?} name={:?}", actor.room, actor.name);
                if let Some(followed) = self.actors.get_mut(actor) {
                    decide(
                        Happened::Subscribed,
                        self.ask_again.keeping(),
                        &mut followed.tries,
                    );
                }
                self.settle_early()?;
            }
            (Purpose::Room(room), RequestEvent::PublishEstablished { forward, .. }) => {
                let room = room.clone();
                self.room_published(token, &room, forward);
            }
            (Purpose::Room(..), RequestEvent::SubscriptionUpdated { response, .. }) => {
                self.tell(token, Command::Say(response));
            }
            // §10.9.1: the update is refused, and the PUBLISH ends with
            // UPDATE_FAILED once its streams are closed (§10.11).
            (Purpose::Room(room), RequestEvent::UpdateRefused { response }) => {
                let room = room.clone();
                self.tell(token, Command::Say(response));
                if self.publishing.is_publish(&room, token) {
                    self.end_publish(&room, publish_done_status::UPDATE_FAILED, false);
                }
            }
            (Purpose::Actor(actor), RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                let end = Happened::Refused {
                    error_code: error.error_code,
                    named_ms: error.retry_wait_ms(),
                };
                self.acted_over(actor, end, &why);
            }
            (Purpose::Watch, RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                let wait = self.watch.refused(error.retry_wait_ms(), Instant::now());
                self.watch_ended(&why, wait);
            }
            (_, RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                self.request_over(token, purpose, Happened::Ended, &why);
            }
            (Purpose::Actor(actor), RequestEvent::SubscriptionDone(done)) => {
                println!(
                    "SUBSCRIPTION_DONE room={:?} name={:?} status={:#x}",
                    actor.room, actor.name, done.status_code
                );
                self.let_go(token, Command::Finish);
                self.acted_over(actor, Happened::Ended, "PUBLISH_DONE");
            }
            (
                _,
                RequestEvent::TrackNotProcessed {
                    refusal: TrackRefusal::Malformed,
                    ..
                },
            ) => {
                if let Some(request) = self.own.get(token).map(|own| own.request) {
                    self.malformed(request);
                }
            }
            (
                _,
                RequestEvent::TrackNotProcessed { .. } | RequestEvent::TooManyNamespaces { .. },
            ) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(
                    token,
                    purpose,
                    Happened::Ended,
                    "the session could not take its answer",
                );
            }
            // LateMessage, and what is no answer to a request of its kind.
            _ => {}
        }
        Ok(None)
    }

    /// An event of the relay's request streams: a FETCH is answered.
    fn on_peer_request(&mut self, event: PeerRequestStreamEvent<Send>) -> Result<()> {
        if let Some(asked) = self
            .requests
            .on_event(&mut self.session, &self.stops, event)?
        {
            self.on_fetch(asked)?;
        }
        Ok(())
    }

    /// A FETCH of the relay for the `garden` of a room: the latest garden it
    /// holds ([`xmoq_garden::rooms::Room::fetch`]).
    fn on_fetch(&mut self, asked: AskedFetch<Send>) -> Result<()> {
        // WHY a Joining FETCH is refused: as the gatherer refuses it, the
        // relay turns a viewer's into a Standalone one before it asks a
        // publisher (`Relay::join` of `crates/relay-core/src/fetch.rs`).
        let FetchRange::Standalone {
            full_track_name,
            start,
            end,
        } = asked.fetch.range.clone()
        else {
            return self.refuse(asked, request_error_code::NOT_SUPPORTED);
        };
        let Some(GardenTrack::Garden { room }) = tracks::track_of(&full_track_name) else {
            return self.refuse(asked, request_error_code::DOES_NOT_EXIST);
        };
        if self.requests.full() {
            return self.refuse(asked, request_error_code::EXCESSIVE_LOAD);
        }
        let group_order = answers::group_order(&asked.fetch);
        let Some(entry) = self.rooms.get(&room) else {
            return self.refuse(asked, request_error_code::DOES_NOT_EXIST);
        };
        let (planned, gardens) = match entry.fetch(start, end, group_order) {
            Ok(planned) => planned,
            Err(code) => {
                println!("FETCH_REFUSED room={room:?} error_code={code:#x}");
                return self.refuse(asked, code);
            }
        };
        let count = gardens.len();
        let pieces = pieces_of(gardens, planned.left_out);
        if !self.requests.fits(&pieces) {
            return self.refuse(asked, request_error_code::EXCESSIVE_LOAD);
        }
        let answer = FetchAnswer {
            end_location: planned.end_location,
            group_order,
            pieces,
        };
        self.requests
            .answer(self.connection, &mut self.session, asked, answer)?;
        println!(
            "FETCH_ANSWERED room={room:?} start={},{} end={},{} gardens={count}",
            start.group, start.object, planned.end_location.group, planned.end_location.object,
        );
        Ok(())
    }

    /// Refuses a FETCH of the relay with `code`.
    fn refuse(&mut self, asked: AskedFetch<Send>, code: u64) -> Result<()> {
        Ok(self.requests.refuse_fetch(&mut self.session, asked, code)?)
    }

    /// What the task of a FETCH said.
    fn fetch_noted(&mut self, note: Note) {
        self.requests.on_note(&mut self.session, &self.stops, note);
    }
}
