//! One `MoQ` Transport session of the gatherer with the relay: what it asks
//! for, what it publishes, and the FETCH requests it answers.
//!
//! WHY the gatherer sends PUBLISH for the `lines` of each room, and not
//! `PUBLISH_NAMESPACE` for `xmoq/chat/<room>`:
//!
//! - The own relay sends a SUBSCRIBE "to all matching publishers" (§9.5),
//!   matching by prefix, and the namespace of a room is a prefix of the
//!   namespace of each of its participants. Announced, the gatherer would
//!   be asked for every `said` of the room, its own SUBSCRIBEs among them
//!   (the relay does not leave the subscriber out of whom it asks:
//!   `Relay::ask_publishers` of `crates/relay-core/src/routing.rs`), and
//!   would refuse each.
//! - A FETCH reaches a PUBLISH's publisher first: "the first source of the
//!   track that is not on its way out, then the first publisher that
//!   announced a matching namespace" (`whom_to_ask` of
//!   `crates/relay-core/src/fetch.rs`), which §9.5 allows ("determines the
//!   available publishers using the same matching rules as SUBSCRIBE",
//!   and those match an Established subscription on the Full Track Name).
//! - With PUBLISH the relay knows the largest line from the gatherer's
//!   `LARGEST_OBJECT` (§10.2.11) before any viewer asks, also after the
//!   gatherer started again; a viewer's `SUBSCRIBE_OK` carries it, and a
//!   Joining FETCH ends there (§10.12.2.1).
//!
//! WHY NOT announce as well, for a viewer that subscribes before a room
//! has a PUBLISH: the first reason holds.
//!
//! **A room is published while somebody is followed in it**, and its
//! PUBLISH ends with `PUBLISH_DONE` `TRACK_ENDED` when the last one leaves
//! (§10.11: "The track is no longer being published"). WHY NOT every room
//! of the history: the own relay keeps 256 requests of one session
//! (`MAX_PEER_REQUESTS` of `xmoq-moqt`), and the rooms of the history only
//! grow (`crates/chat/src/rooms.rs`). What that costs: a FETCH for a room
//! nobody is in reaches no publisher, and the own relay refuses it with
//! `DOES_NOT_EXIST` (`ask_for` of `crates/relay-core/src/fetch.rs`, where
//! nobody is left to ask); its lines are in the store, and a FETCH gets them again
//! once somebody is in the room.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use xmoq_chat::args::Rooms as Taken;
use xmoq_chat::gather::Left;
use xmoq_chat::said_again::Settings as AskAgain;
use xmoq_moqt::params::SubscriptionFilter;

use anyhow::{Result, anyhow, bail};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;
use xmoq_chat::gather::{Admitted, Gatherer};
use xmoq_chat::react::{Reacting, ReactionDropped};
use xmoq_chat_tracks::names::{
    self, ChatTrack, Person, PersonTrack, Room, RoomTrack, person_of, person_track, room_track,
    track_of,
};
use xmoq_chat_tracks::reactions::Present;
use xmoq_moqt::data::ObjectStatus;
use xmoq_moqt::early::{Arrived, EarlyObjects, held_bytes};
use xmoq_moqt::immutable::TrackRefusal;
use xmoq_moqt::message::{
    ControlMessage, FetchRange, ReasonPhrase, publish_done_status, request_error_code,
};
use xmoq_moqt::name::{Location, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter};
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
use xmoq_room_token::{Role as TokenRole, RoomKey};

use xmoq_again::next_session::{Pause, new_url};
use xmoq_publisher::answers::{self, Bounds, MAX_ANSWERED_AT_ONCE, Note};
use xmoq_publisher::fetch::{pieces_of, plan};
use xmoq_publisher::own::OwnRequests;
use xmoq_publisher::publication::{Ending, FIRST_TRACK_ALIAS, Publication};
use xmoq_publisher::requests::{AskedFetch, FetchAnswer, PeerRequests};
use xmoq_publisher::streams::{COMMAND_QUEUE, Command, GroupToSend, Heard, Sending, send_group};
use xmoq_publisher::{shown, sleep_until};
use xmoq_subscriber::kept::{Becomes, Happened, Tries, decide};
use xmoq_subscriber::namespaces::KeptNamespace;

use crate::storage::Store;

type Send = <WtConnection as Connection>::Send;

/// How long a request may wait for the relay to allow its stream, as the
/// Rust client waits (`STREAM_OPEN_WAIT_MS`). Provisional.
const OPEN_WAIT: Duration = Duration::from_secs(10);

/// Events of the relay's unidirectional streams not taken yet; the readers
/// wait when it is full, and flow control holds the relay back.
/// Provisional.
const UNI_QUEUE: usize = 64;

/// Requests the relay opened that are not taken yet; as above.
const PEER_REQUEST_QUEUE: usize = 16;

/// What the tasks of the gatherer's request streams said and the loop did
/// not take yet; they wait when it is full. Provisional.
const HEARD_QUEUE: usize = 256;

/// Lines of a room that wait for its `PUBLISH_OK`. Past it a line is not
/// sent live; it is in the store, and a FETCH gets it. Provisional.
const LINES_WAITING_FOR_A_ROOM: usize = 64;

/// The Objects of a room's `reactions` or `present` that wait for its
/// `PUBLISH_OK`, the oldest dropped first. WHY fewer than of `lines`: each
/// says the state of a while or of the room, which the next says anew.
/// **Provisional.**
const COUNTS_WAITING_FOR_A_ROOM: usize = 4;

/// An Object of a room's track that waits for its `PUBLISH_OK`: its Group
/// ID and its payload.
type Waiting = (u64, Vec<u8>);

/// The tracks the gatherer publishes for a room.
const ROOM_TRACKS: [RoomTrack; 3] = [RoomTrack::Lines, RoomTrack::Reactions, RoomTrack::Present];

/// The tracks the gatherer subscribes to of a participant.
const PERSON_TRACKS: [PersonTrack; 2] = [PersonTrack::Said, PersonTrack::Reacted];

/// What the log calls a track of a room. WHY `lines` is `ROOM`: what the log
/// said before the room had other tracks, which the interop runs wait for.
fn room_word(track: RoomTrack) -> &'static str {
    match track {
        RoomTrack::Lines => "ROOM",
        RoomTrack::Reactions => "REACTIONS",
        RoomTrack::Present => "PRESENT",
    }
}

/// What the log puts before the words of a participant's track. WHY none for
/// `said`: as for `ROOM`.
fn person_word(track: PersonTrack) -> &'static str {
    match track {
        PersonTrack::Said => "",
        PersonTrack::Reacted => "REACTED_",
    }
}

/// Subgroup streams of lines being written at once. Past it a line is not
/// sent live; as above. WHY a limit: a relay that takes nothing would have
/// a task for every line. Provisional.
const LINES_BEING_SENT: usize = 256;

/// The most bytes of payload the answers to FETCH requests that are being
/// written hold together (`Held` of `xmoq-publisher`).
///
/// Not a flag: with `--fetch-lines` and `--text-bytes` at their most, one
/// answer could be 100,000 lines of 49 KiB, and 32 are answered at once.
/// WHY 64 MiB: a thousand lines of the default 512 bytes are under 1 MiB,
/// so 64 answers of that size fit, and it is a quarter of what the own relay
/// lets the fetch streams it writes keep (`MAX_BYTES_HELD_BEYOND_THE_CACHE`).
/// The lines are read from the store one by one and the reading stops at
/// the bound, so no more than that is read for an answer that is refused.
/// **Provisional.**
const MAX_BYTES_HELD: usize = 64 * 1024 * 1024;

/// How a session ended without an error.
#[derive(Debug)]
pub enum SessionEnd {
    /// The gatherer was told to stop.
    Stopped,
    /// The relay sent GOAWAY on the control stream (§10.4), with the URL
    /// to go to, or none for the same.
    Moved(Option<String>),
}

/// A participant that is followed in this session: its `said` and its
/// `reacted`.
struct Followed {
    said: Subscription,
    reacted: Subscription,
}

/// The subscription to one track of a followed participant.
struct Subscription {
    /// The token of its SUBSCRIBE, while it has one.
    token: Option<u64>,
    /// When it is asked again after it is over.
    tries: Tries,
}

impl Followed {
    /// Neither track is subscribed to yet.
    fn new(settings: AskAgain) -> Self {
        let subscription = || Subscription {
            token: None,
            tries: settings.tries(),
        };
        Self {
            said: subscription(),
            reacted: subscription(),
        }
    }

    fn of(&self, track: PersonTrack) -> &Subscription {
        match track {
            PersonTrack::Said => &self.said,
            PersonTrack::Reacted => &self.reacted,
        }
    }

    fn of_mut(&mut self, track: PersonTrack) -> &mut Subscription {
        match track {
            PersonTrack::Said => &mut self.said,
            PersonTrack::Reacted => &mut self.reacted,
        }
    }
}

/// What a request of the gatherer is for.
#[derive(Debug, Clone)]
enum Purpose {
    /// A `SUBSCRIBE_NAMESPACE`: for `xmoq/chat`, or for `xmoq/chat/<room>`
    /// of each `--room`, by its place in [`Gathering::watches`].
    Watch(usize),
    /// The SUBSCRIBE for a track of a participant.
    Person(Person, PersonTrack),
    /// The PUBLISH of a track of a room.
    Room(Room, RoomTrack),
}

/// Says that the `SUBSCRIBE_NAMESPACE` for `prefix` is asked for again
/// `wait_ms` from now.
fn watch_ended(prefix: &TrackNamespace, why: &str, wait_ms: u64) {
    println!(
        "WATCH_ENDED prefix={} why={why:?} again_in_ms={wait_ms}",
        shown(prefix)
    );
}

/// Everything of one session.
struct Gathering<'a> {
    connection: &'a Arc<WtConnection>,
    store: &'a Store,
    gatherer: &'a mut Gatherer,
    session: Session,
    stops: StreamStops,
    own: OwnRequests<Purpose>,
    /// The tracks of each room the gatherer numbers, with the Objects that
    /// wait for their `PUBLISH_OK` by Group ID.
    published: HashMap<(Room, RoomTrack), Publication<Waiting>>,
    /// The last Group ID of each room's `reactions` and `present`.
    last_group: HashMap<(Room, RoomTrack), u64>,
    /// The reactions of the while under way.
    reacting: Reacting,
    next_track_alias: u64,
    /// Each participant that is followed.
    people: HashMap<Person, Followed>,
    /// The tracks of participants that are to be asked for again, and when.
    /// WHY apart from [`Self::people`]: the loop looks for the earliest
    /// with every event, and few wait at once out of up to 4,096 followed.
    asking_again: HashMap<(Person, PersonTrack), Instant>,
    /// `--ask-again-times`, `--ask-again-ms`.
    ask_again: AskAgain,
    /// Objects of `said` read before the `SUBSCRIBE_OK` that gives their
    /// Track Alias, by that alias, with their payload (`xmoq_moqt::early`).
    early: EarlyObjects<(u64, Vec<u8>)>,
    /// The requests of the relay: the FETCH requests being answered, and
    /// those refused.
    requests: PeerRequests,
    /// The PUBLISH requests that wait for their streams to be closed, by
    /// token, with their room and track, and where the loop is told that
    /// they are.
    ending: HashMap<u64, ((Room, RoomTrack), Ending)>,
    drained: mpsc::Sender<u64>,
    watches: Vec<KeptNamespace>,
    /// The rooms the gatherer takes (`--room`).
    taken: &'a Taken,
    /// `--auth-key-hex`.
    key: Option<&'a RoomKey>,
    sends: JoinSet<()>,
    began: Instant,
}

/// The gatherer's clock since the epoch, in milliseconds.
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
        RequestFault::Usage(error) => anyhow!("the gatherer used its session wrongly: {error}"),
    }
}

/// The prefixes of the `SUBSCRIBE_NAMESPACE` requests for the rooms taken.
fn prefixes(taken: &Taken) -> Result<Vec<TrackNamespace>> {
    Ok(match taken {
        Taken::Every => vec![names::prefix()?],
        Taken::These(rooms) => rooms
            .iter()
            .map(names::room_namespace)
            .collect::<Result<_, _>>()?,
    })
}

/// Runs one session on `connection` until it ends.
#[expect(
    clippy::too_many_lines,
    reason = "the readers, the state and the one loop of the session, which share their queues"
)]
pub async fn gather(
    connection: &Arc<WtConnection>,
    kept: &mut client::UntilClosed<WtConnection>,
    store: &Store,
    (gatherer, rooms_taken, key): (&mut Gatherer, &Taken, Option<&RoomKey>),
    ask_again: AskAgain,
    stop: &mut watch::Receiver<bool>,
) -> Result<SessionEnd> {
    let stops = StreamStops::default();
    let (uni_queue, mut uni) = mpsc::channel(UNI_QUEUE);
    let stream_config = StreamConfig {
        subgroup_buffer_limit: xmoq_chat::args::PAYLOAD_MOST,
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
    let reacting = Reacting::new(gatherer.limits());
    let mut gathering = Gathering {
        connection,
        store,
        gatherer,
        session: Session::new(Role::Client, Transport::WebTransport),
        stops,
        own: OwnRequests::new(heard, OPEN_WAIT),
        published: HashMap::new(),
        last_group: HashMap::new(),
        reacting,
        next_track_alias: FIRST_TRACK_ALIAS,
        people: HashMap::new(),
        asking_again: HashMap::new(),
        ask_again,
        early: EarlyObjects::default(),
        requests: PeerRequests::new(Bounds {
            at_once: MAX_ANSWERED_AT_ONCE,
            bytes: MAX_BYTES_HELD,
            say_queue: COMMAND_QUEUE,
        }),
        ending: HashMap::new(),
        drained,
        watches: prefixes(rooms_taken)?
            .into_iter()
            .map(|prefix| KeptNamespace::new(prefix, Pause::SERVING))
            .collect(),
        taken: rooms_taken,
        key,
        sends: JoinSet::new(),
        began: Instant::now(),
    };

    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    // The relay looks at no namespace or role of the token of a SETUP.
    let token = gathering.token(TokenRole::Sub);
    let options = client::setup_options(token.as_deref(), implementation)?;
    let set_up = client::setup(
        connection.as_ref(),
        &mut gathering.session,
        &mut uni,
        &mut kept.control,
        options,
        client::SETUP_WAIT,
    )
    .await?;
    println!("SETUP_OK");
    // What came before the SETUP (§3.3) is taken first.
    for event in set_up.early {
        if let Some(ended) = gathering.on_uni(event)? {
            return Ok(ended);
        }
    }
    // Who is in the rooms is what this session is told; the numbering of
    // the rooms goes on.
    gathering.gatherer.leave_everybody();
    for index in 0..gathering.watches.len() {
        gathering.watch(index)?;
    }

    let batch_ms = gathering.gatherer.limits().reaction_batch_ms;
    let mut batch = tokio::time::interval(Duration::from_millis(batch_ms));
    // A loop that was held up counts one longer while, not several at once.
    batch.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let ended = loop {
        // WHY taken before the select: its arms borrow parts of the state
        // mutably, the task notes of the FETCH answers among them.
        let (next_watch, next_asked) = (gathering.next_watch(), gathering.next_asked());
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break SessionEnd::Stopped;
                }
            }
            event = uni.recv() => {
                let event = event.ok_or_else(|| anyhow!("the connection is gone"))?;
                if let Some(ended) = gathering.on_uni(event)? {
                    break ended;
                }
            }
            event = peer.recv() => {
                let event = event.ok_or_else(|| anyhow!("the connection is gone"))?;
                gathering.on_peer_request(event)?;
            }
            Some((token, heard)) = heard_by_loop.recv() => {
                if let Some(ended) = gathering.on_heard(token, heard)? {
                    break ended;
                }
            }
            note = gathering.requests.next_note() => gathering.fetch_noted(note),
            Some(token) = drained_rooms.recv() => gathering.publish_done(token)?,
            () = sleep_until(next_watch) => gathering.watch_again()?,
            () = sleep_until(next_asked) => gathering.ask_again()?,
            _ = batch.tick() => gathering.send_reactions()?,
            Some(_) = gathering.sends.join_next() => {}
        }
    };
    Ok(ended)
}

impl Gathering<'_> {
    /// A token of the gatherer's own for `role` in every namespace of the
    /// chat (`xmoq/chat/*`) from now, as an AUTHORIZATION TOKEN
    /// ([`RoomKey::authorization_token`]); `None` without `--auth-key-hex`.
    ///
    /// WHY made anew for each request: the relay checks a request when it
    /// comes (`crates/relay/src/auth.rs`), and a gatherer runs for longer
    /// than any one token, so that one made at the start would turn its
    /// later requests down. WHY `xmoq/chat/*` and not a token a room:
    /// the gatherer follows every room it takes, and the prefix it asks
    /// for (`SUBSCRIBE_NAMESPACE xmoq/chat`) is under no one room.
    fn token(&self, role: TokenRole) -> Option<Vec<u8>> {
        let key = self.key?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let namespace = format!("{}/*", names::PREFIX.join("/"));
        Some(key.authorization_token(&namespace, role, now))
    }

    /// [`Self::token`] as the parameter of a request.
    fn token_parameter(&self, role: TokenRole) -> Option<Parameter> {
        self.token(role).map(Parameter::AuthorizationToken)
    }

    /// Asks for the namespaces under the prefix of a watch.
    fn watch(&mut self, index: usize) -> Result<()> {
        let Some(watch) = self.watches.get_mut(index) else {
            return Ok(());
        };
        watch.asked(Instant::now());
        let prefix = watch.prefix().clone();
        let (request, message) = self.session.subscribe_namespace(
            prefix,
            self.token_parameter(TokenRole::Sub).into_iter().collect(),
        )?;
        self.open(Purpose::Watch(index), request, message, &[]);
        Ok(())
    }

    /// When the next watch that ended is to be asked for again.
    fn next_watch(&self) -> Option<Instant> {
        self.watches
            .iter()
            .filter_map(KeptNamespace::again_at)
            .min()
    }

    /// Asks again for every watch whose time has come.
    fn watch_again(&mut self) -> Result<()> {
        let now = Instant::now();
        let due: Vec<usize> = (0..self.watches.len())
            .filter(|index| self.watches.get(*index).is_some_and(|watch| watch.due(now)))
            .collect();
        for index in due {
            self.watch(index)?;
        }
        Ok(())
    }

    /// Opens the stream of a request of the gatherer.
    fn open(
        &mut self,
        purpose: Purpose,
        request: RequestId,
        message: ControlMessage,
        parameters: &[Parameter],
    ) -> u64 {
        self.own
            .open(self.connection, purpose, request, message, parameters)
    }

    /// Tells the task of a request stream something.
    ///
    /// WHY `try_send` and not a wait: the loop must not wait on one stream.
    /// WHY no command is ever dropped: among them are the answers to the
    /// relay's `REQUEST_UPDATE`s, and §10.9 has "The receiver of a
    /// `REQUEST_UPDATE` MUST respond with exactly one `REQUEST_OK` or
    /// `REQUEST_ERROR`". Where the queue is full ([`xmoq_publisher::streams::COMMAND_QUEUE`]),
    /// the relay takes nothing on the stream and sends more than it reads:
    /// the request is cancelled with `EXCESSIVE_LOAD` (§3.3.3), as the own
    /// relay ends a FETCH with too many updates waiting.
    fn tell(&mut self, token: u64, command: Command) {
        let full = self.own.tell(token, command).is_err();
        if full {
            self.cancel_request(token, stream_reset_code::EXCESSIVE_LOAD);
        }
    }

    /// Cancels a request (§3.3.2) before what waits in its queue, and has
    /// what it was for go on without it.
    fn cancel_request(&mut self, token: u64, code: u32) {
        if let Some(purpose) = self.let_go(token, Command::Cancel(code)) {
            self.request_over(token, &purpose, "its queue was full");
        }
    }

    /// Ends a request in the session and on its stream, and forgets it.
    fn let_go(&mut self, token: u64, command: Command) -> Option<Purpose> {
        self.own.let_go(&mut self.session, token, command)
    }

    /// Publishes every track of a room ([`ROOM_TRACKS`]).
    fn publish_room(&mut self, room: &Room) -> Result<()> {
        for track in ROOM_TRACKS {
            self.publish(room, track)?;
        }
        Ok(())
    }

    /// Publishes a track of a room, unless it is published or waits to be
    /// published again.
    fn publish(&mut self, room: &Room, track: RoomTrack) -> Result<()> {
        let most_waiting = match track {
            RoomTrack::Lines => LINES_WAITING_FOR_A_ROOM,
            RoomTrack::Reactions | RoomTrack::Present => COUNTS_WAITING_FOR_A_ROOM,
        };
        let entry = self
            .published
            .entry((room.clone(), track))
            .or_insert_with(|| Publication::new(most_waiting));
        if !entry.to_publish(Instant::now()) {
            return Ok(());
        }
        let track_alias = self.next_track_alias;
        self.next_track_alias += 1;
        // §10.2.11: the largest line the gatherer has, which the relay gives
        // its subscribers in SUBSCRIBE_OK, and where a Joining FETCH ends.
        // WHY none for the counts: nothing of them is kept, and no FETCH
        // gets any (decided on 2026-10-26).
        let largest = match track {
            RoomTrack::Lines => self.gatherer.rooms.largest(room),
            RoomTrack::Reactions | RoomTrack::Present => None,
        };
        let parameters: Vec<Parameter> = largest
            .map(|group| Parameter::LargestObject(Location { group, object: 0 }))
            .into_iter()
            .chain(self.token_parameter(TokenRole::Pub))
            .collect();
        let published = self.session.publish(
            room_track(room, track)?,
            track_alias,
            parameters.clone(),
            vec![],
        );
        let Ok((request, message)) = published else {
            // The PUBLISH before it still waits for its streams to be
            // closed; it is published again once that is over.
            println!(
                "{}_NOT_PUBLISHED room={room:?} why=\"the PUBLISH before is ending\"",
                room_word(track),
                room = room.to_string()
            );
            return Ok(());
        };
        let purpose = Purpose::Room(room.clone(), track);
        let token = self.open(purpose, request, message, &parameters);
        if let Some(entry) = self.published.get_mut(&(room.clone(), track)) {
            entry.published(token, track_alias);
        }
        Ok(())
    }

    /// The PUBLISH of a room's track is over; the next Object publishes it
    /// again. Does nothing where `token` is no longer its PUBLISH.
    fn unpublish(&mut self, room: &Room, track: RoomTrack, token: u64, failed: bool) {
        if let Some(entry) = self.published.get_mut(&(room.clone(), track))
            && entry.unpublished(token, failed, Instant::now())
        {
            println!(
                "{}_UNPUBLISHED room={room:?}",
                room_word(track),
                room = room.to_string()
            );
        }
    }

    /// Ends the PUBLISH of every track of a room ([`Self::end_publish`]).
    fn end_room(&mut self, room: &Room, status: u64, forget: bool) {
        for track in ROOM_TRACKS {
            self.end_publish(room, track, status, forget);
        }
    }

    /// Ends the PUBLISH of a room's track with `status`: no Object is sent
    /// on it from now on, and its `PUBLISH_DONE` goes once the streams being
    /// sent are closed ([`Self::publish_done`]). `forget`: the room is
    /// numbered no more, and its state goes.
    fn end_publish(&mut self, room: &Room, track: RoomTrack, status: u64, forget: bool) {
        let key = (room.clone(), track);
        let ending = self
            .published
            .get_mut(&key)
            .and_then(|entry| entry.end(status));
        if forget {
            self.published.remove(&key);
            self.last_group.remove(&key);
        }
        let Some(ending) = ending else {
            return;
        };
        let drained = self.drained.clone();
        let when_drained = ending.when_drained();
        tokio::spawn(async move {
            let _ = drained.send(when_drained.await).await;
        });
        self.ending.insert(ending.token, (key, ending));
    }

    /// Every stream of an ending PUBLISH is closed: its `PUBLISH_DONE`, with
    /// the number of streams it opened (§10.11), and a FIN. Where the room
    /// is numbered again meanwhile, it is published anew.
    fn publish_done(&mut self, token: u64) -> Result<()> {
        let Some(((room, track), ending)) = self.ending.remove(&token) else {
            return Ok(());
        };
        if let Some(own) = self.own.get(token) {
            let streams = ending.streams();
            let done = self.session.publish_done(
                own.request,
                ending.status,
                streams,
                ReasonPhrase::default(),
            )?;
            self.tell(token, Command::Say(done));
            self.let_go(token, Command::Finish);
            println!(
                "{}_DONE room={room:?} status={:#x} streams={streams}",
                room_word(track),
                ending.status,
                room = room.to_string()
            );
        }
        if self.gatherer.rooms.knows(&room) {
            if let Some(entry) = self.published.get_mut(&(room.clone(), track)) {
                entry.may_publish_again();
            }
            self.publish(&room, track)?;
        }
        Ok(())
    }

    /// Subscribes to a track of a participant: what it says, or its
    /// reactions.
    ///
    /// WHY the Largest Object filter (§5.1.2: "The filter Start Location is
    /// {Largest Object.Group, Largest Object.Object + 1}"): without a filter
    /// "Subscriptions without a filter pass all Objects published or
    /// received via upstream subscriptions" (§5.1.2, §10.2.9: "If omitted
    /// from SUBSCRIBE ... the subscription is unfiltered"), and a relay that
    /// still has a participant's lines would pass them again to a gatherer
    /// that connects anew, which numbers what comes and would keep each
    /// twice; a reaction would be counted again. WHY NOT remember the Group
    /// IDs taken of each participant: a participant that comes again numbers
    /// its Groups anew (the own Rust client from 0), and its new lines would
    /// be taken for old ones. What a participant said while the gatherer was
    /// away is lost either way.
    fn subscribe(&mut self, person: &Person, track: PersonTrack) -> Result<()> {
        let mut parameters = vec![
            Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
            Parameter::Forward(true),
            Parameter::SubscriberPriority(128),
            Parameter::GroupOrder(GroupOrder::Ascending),
        ];
        parameters.extend(self.token_parameter(TokenRole::Sub));
        self.asking_again.remove(&(person.clone(), track));
        let Ok((request, message)) = self
            .session
            .subscribe(person_track(person, track)?, parameters.clone())
        else {
            // A subscription to the track is there already.
            println!(
                "{}NOT_SUBSCRIBED room={:?} name={:?} why=\"a subscription is there already\"",
                person_word(track),
                person.room.to_string(),
                person.name
            );
            return Ok(());
        };
        let purpose = Purpose::Person(person.clone(), track);
        let token = self.open(purpose, request, message, &parameters);
        if let Some(followed) = self.people.get_mut(person) {
            followed.of_mut(track).token = Some(token);
        }
        Ok(())
    }

    /// A track of a participant that stays announced is over: it is asked
    /// for again after a pause, or not until its NAMESPACE comes again
    /// (`kept::decide` of `xmoq-subscriber`, by [`AskAgain::keeping`]).
    fn track_over(&mut self, person: &Person, track: PersonTrack, end: Happened, why: &str) {
        let word = person_word(track);
        println!(
            "{word}NOT_SUBSCRIBED room={:?} name={:?} why={why:?}",
            person.room.to_string(),
            person.name
        );
        // Not followed: its NAMESPACE_DONE came.
        let Some(followed) = self.people.get_mut(person) else {
            return;
        };
        let subscription = followed.of_mut(track);
        subscription.token = None;
        match decide(end, self.ask_again.keeping(track), &mut subscription.tries) {
            Becomes::Again(pause) => {
                println!(
                    "{word}ASKING_AGAIN room={:?} name={:?} in_ms={pause}",
                    person.room.to_string(),
                    person.name
                );
                self.asking_again.insert(
                    (person.clone(), track),
                    Instant::now() + Duration::from_millis(pause),
                );
            }
            Becomes::GivenUp | Becomes::Subscribed => println!(
                "{word}NOT_ASKING_AGAIN room={:?} name={:?}",
                person.room.to_string(),
                person.name
            ),
        }
    }

    /// The subscription to a track of a participant is established: the
    /// Objects that came before it are taken.
    fn track_established(&mut self, person: &Person, track: PersonTrack) -> Result<()> {
        println!(
            "{}SUBSCRIBED room={:?} name={:?}",
            person_word(track),
            person.room.to_string(),
            person.name
        );
        if let Some(followed) = self.people.get_mut(person) {
            let keeping = self.ask_again.keeping(track);
            decide(
                Happened::Subscribed,
                keeping,
                &mut followed.of_mut(track).tries,
            );
        }
        self.settle_early()
    }

    /// The Objects held for an alias the session knows now are taken, in the
    /// order they came, and the streams of those no `SUBSCRIBE` can give any
    /// more are stopped (`xmoq_moqt::early`).
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

    /// When the next track of a participant is to be asked for again.
    fn next_asked(&self) -> Option<Instant> {
        self.asking_again.values().min().copied()
    }

    /// Asks again for every track of a participant whose time has come.
    fn ask_again(&mut self) -> Result<()> {
        let now = Instant::now();
        let due: Vec<(Person, PersonTrack)> = self
            .asking_again
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(asked, _)| asked.clone())
            .collect();
        for (person, track) in due {
            self.asking_again.remove(&(person.clone(), track));
            if self
                .people
                .get(&person)
                .is_some_and(|followed| followed.of(track).token.is_none())
            {
                self.subscribe(&person, track)?;
            }
        }
        Ok(())
    }

    /// A NAMESPACE under `xmoq/chat`.
    fn on_namespace(&mut self, namespace: &TrackNamespace) -> Result<()> {
        let Some(person) = person_of(namespace).filter(|person| self.taken.has(&person.room))
        else {
            println!("NAMESPACE_IGNORED namespace={}", shown(namespace));
            return Ok(());
        };
        if !self.gatherer.rooms.knows(&person.room) {
            let next = self.store.next_number(&person.room)?;
            if !self.gatherer.rooms.add(&person.room, next) {
                println!(
                    "PARTICIPANT_IGNORED room={:?} name={:?} why=too-many-rooms",
                    person.room.to_string(),
                    person.name
                );
                return Ok(());
            }
        }
        self.publish_room(&person.room)?;
        let admitted = self.gatherer.admit(&person);
        if matches!(admitted, Admitted::Now | Admitted::Already) {
            // A NAMESPACE that comes again counts the times anew.
            let settings = self.ask_again;
            let followed = self
                .people
                .entry(person.clone())
                .or_insert_with(|| Followed::new(settings));
            for track in PERSON_TRACKS {
                followed.of_mut(track).tries = settings.tries();
            }
        }
        match admitted {
            Admitted::Now => {
                println!(
                    "PARTICIPANT room={:?} name={:?}",
                    person.room.to_string(),
                    person.name
                );
                for track in PERSON_TRACKS {
                    self.subscribe(&person, track)?;
                }
                self.send_present(&person.room)
            }
            // Followed: the tracks with no subscription in this session.
            Admitted::Already => {
                for track in PERSON_TRACKS {
                    if self
                        .people
                        .get(&person)
                        .is_some_and(|followed| followed.of(track).token.is_none())
                    {
                        self.subscribe(&person, track)?;
                    }
                }
                Ok(())
            }
            Admitted::TooManyPeople | Admitted::NoRoom => {
                println!(
                    "PARTICIPANT_IGNORED room={:?} name={:?} why=too-many-people",
                    person.room.to_string(),
                    person.name
                );
                Ok(())
            }
        }
    }

    /// A `NAMESPACE_DONE` under `xmoq/chat`: the participant is gone.
    fn on_namespace_done(&mut self, namespace: &TrackNamespace) -> Result<()> {
        let Some(person) = person_of(namespace) else {
            return Ok(());
        };
        for track in PERSON_TRACKS {
            self.asking_again.remove(&(person.clone(), track));
        }
        if let Some(followed) = self.people.remove(&person) {
            for token in [followed.said.token, followed.reacted.token]
                .into_iter()
                .flatten()
            {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
            }
        }
        self.reacting.leave(&person);
        let left = self.gatherer.leave(&person);
        if left != Left::NotFollowed {
            println!(
                "PARTICIPANT_GONE room={:?} name={:?}",
                person.room.to_string(),
                person.name
            );
            self.send_present(&person.room)?;
        }
        if left == Left::RoomEmpty {
            self.reacting.forget(&person.room);
            self.end_room(&person.room, publish_done_status::TRACK_ENDED, true);
        }
        Ok(())
    }

    /// An Object of `said`: a line.
    fn on_said(&mut self, person: &Person, payload: &[u8]) -> Result<()> {
        let now_ms = u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX);
        let line = match self.gatherer.take(person, payload, now_ms, wall_ms()) {
            Ok(line) => line,
            Err(dropped) => {
                println!(
                    "LINE_DROPPED room={:?} name={:?} why={} dropped={}",
                    person.room.to_string(),
                    person.name,
                    dropped.word(),
                    self.gatherer.dropped(dropped.word())
                );
                return Ok(());
            }
        };
        if let Err(error) = self.store.keep(&person.room, &line) {
            self.gatherer.rooms.give_back(&person.room, line.number);
            println!(
                "LINE_DROPPED room={:?} name={:?} why=store error={error:#}",
                person.room.to_string(),
                person.name
            );
            return Ok(());
        }
        println!(
            "LINE room={:?} number={} name={:?} text_bytes={}",
            person.room.to_string(),
            line.number,
            line.name,
            line.text.len()
        );
        self.send_live(&person.room, RoomTrack::Lines, line.number, line.payload())
    }

    /// An Object of `reacted`: a reaction, counted in its room's while.
    fn on_reacted(&mut self, person: &Person, payload: &[u8]) {
        let now_ms = u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX);
        let counted = if self.gatherer.follows(person) {
            self.reacting.react(person, payload, now_ms)
        } else {
            Err(ReactionDropped::NotFollowed)
        };
        if let Err(dropped) = counted {
            println!(
                "REACTION_DROPPED room={:?} name={:?} why={}",
                person.room.to_string(),
                person.name,
                dropped.word()
            );
        }
    }

    /// An Object of a track the gatherer subscribed to under `track_alias`.
    fn on_object(&mut self, track_alias: u64, payload: &[u8]) -> Result<()> {
        let track = match self.session.subscribed_alias(track_alias) {
            SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => {
                track_of(track)
            }
            SubscribedAlias::Unknown | SubscribedAlias::Retired => None,
        };
        match track {
            Some(ChatTrack::OfPerson(person, PersonTrack::Said)) => self.on_said(&person, payload),
            Some(ChatTrack::OfPerson(person, PersonTrack::Reacted)) => {
                self.on_reacted(&person, payload);
                Ok(())
            }
            // The gatherer subscribes to no track of a room.
            Some(ChatTrack::OfRoom(..)) | None => Ok(()),
        }
    }

    /// Sends Group `group_id` of a room's track, one Object of `payload`,
    /// or has it wait for the track's `PUBLISH_OK`.
    fn send_live(
        &mut self,
        room: &Room,
        track: RoomTrack,
        group_id: u64,
        payload: Vec<u8>,
    ) -> Result<()> {
        let key = (room.clone(), track);
        let is_live = self.published.get(&key).is_some_and(Publication::live);
        if !is_live {
            self.publish(room, track)?;
            if let Some(entry) = self.published.get_mut(&key) {
                entry.wait((group_id, payload));
            }
            return Ok(());
        }
        let Some(entry) = self.published.get(&key) else {
            return Ok(());
        };
        // What the log calls an Object of each track; `LINE` as before.
        let what = match track {
            RoomTrack::Lines => "LINE",
            RoomTrack::Reactions => "REACTIONS",
            RoomTrack::Present => "PRESENT",
        };
        if self.sends.len() >= LINES_BEING_SENT {
            println!(
                "{what}_NOT_SENT room={room:?} number={group_id} why=too-many-at-once",
                room = room.to_string()
            );
            return Ok(());
        }
        let sending = GroupToSend {
            track_alias: entry.track_alias(),
            group_id,
            payload,
            opened: Arc::clone(entry.opened()),
        };
        let counted = Sending::begin(entry.sending());
        let connection = Arc::clone(self.connection);
        let room = room.to_string();
        self.sends.spawn(async move {
            let _counted = counted;
            let number = sending.group_id;
            if let Err(why) = send_group(connection, sending).await {
                println!("{what}_NOT_SENT room={room:?} number={number} why={why:?}");
            }
        });
        Ok(())
    }

    /// The next Group ID of a room's `reactions` or `present`: the
    /// gatherer's clock in milliseconds, or one more than the last where
    /// that is not more. WHY the clock: a PUBLISH of a later session numbers
    /// on above what this one sent, with nothing kept, while the clock does
    /// not go back; a relay would take Groups below those it has for old.
    fn next_group(&mut self, room: &Room, track: RoomTrack) -> u64 {
        let key = (room.clone(), track);
        let next = self
            .last_group
            .get(&key)
            .map_or(0, |last| last.saturating_add(1))
            .max(wall_ms());
        self.last_group.insert(key, next);
        next
    }

    /// Publishes how many are in a room on its `present`.
    fn send_present(&mut self, room: &Room) -> Result<()> {
        let present = Present {
            count: self.gatherer.present(room),
            at: wall_ms(),
        };
        println!(
            "PRESENT room={room:?} count={}",
            present.count,
            room = room.to_string()
        );
        let group_id = self.next_group(room, RoomTrack::Present);
        self.send_live(room, RoomTrack::Present, group_id, present.payload())
    }

    /// The end of a while: what each room counted goes on its `reactions`.
    fn send_reactions(&mut self) -> Result<()> {
        for (room, reactions) in self.reacting.take(wall_ms()) {
            if !self.gatherer.rooms.knows(&room) {
                continue;
            }
            println!(
                "REACTIONS room={room:?} kinds={} reactions={}",
                reactions.counts.len(),
                reactions.counts.iter().map(|(_, count)| count).sum::<u64>(),
                room = room.to_string()
            );
            let group_id = self.next_group(&room, RoomTrack::Reactions);
            self.send_live(&room, RoomTrack::Reactions, group_id, reactions.payload())?;
        }
        Ok(())
    }

    /// An event of the relay's unidirectional streams.
    fn on_uni(&mut self, event: UniEvent) -> Result<Option<SessionEnd>> {
        // What a SUBSCRIBE that ended meanwhile left held is let go of before
        // what came after it.
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
            // The gatherer sends no FETCH: the session says what such a
            // stream is (`Session::on_fetch_stream`), and it is stopped.
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

    /// What the task of one of the gatherer's request streams heard.
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
                self.request_over(token, &purpose, &format!("not sent: {why}"));
                Ok(None)
            }
            Heard::NotAnswered => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, &purpose, "not answered in time");
                Ok(None)
            }
            Heard::Message(message) => {
                let event = self
                    .session
                    .on_request_message(request, message)
                    .map_err(fault)?;
                self.on_answer(token, &purpose, event)
            }
            // §5.1: a subscriber that only finishes its direction of a
            // PUBLISH's stream has not ended the subscription.
            Heard::Closed(RequestClose::Fin) if matches!(purpose, Purpose::Room(..)) => {
                println!("PUBLISH_PEER_FINISHED_ITS_DIRECTION");
                Ok(None)
            }
            Heard::Closed(how) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, &purpose, &format!("its stream ended ({how})"));
                Ok(None)
            }
            // §5.1: a subscriber ends a subscription that PUBLISH made with
            // STOP_SENDING.
            Heard::Stopped(code) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, &purpose, &format!("stopped ({code:?})"));
                Ok(None)
            }
            Heard::Failed(RequestReadError::Peer(error)) => Err(error.into()),
            Heard::Failed(RequestReadError::Lost) => bail!("the connection is gone"),
        }
    }

    /// The relay sent GOAWAY on the stream of a request of the gatherer: the
    /// request alone is asked again, on this session, as after its end
    /// (§10.4: "the endpoint SHOULD re-issue that specific request ... and
    /// close the old request stream using the appropriate mechanism (e.g.
    /// FIN, stream reset, or `PUBLISH_DONE`)"). A subscription is cancelled
    /// (§5.1), and the PUBLISH of a room ends with `PUBLISH_DONE`
    /// `GOING_AWAY` once its streams are closed (§10.11) and is sent again.
    ///
    /// FIXME: a New Session URI names another session for the request, and
    /// the request is asked again on this one (§10.4: "on a session at the
    /// specified URI"); the gatherer keeps one session.
    fn request_moved(&mut self, token: u64, purpose: &Purpose) {
        match purpose {
            Purpose::Room(room, track) => {
                if self
                    .published
                    .get(&(room.clone(), *track))
                    .is_some_and(|entry| entry.publish() == Some(token))
                {
                    let room = room.clone();
                    self.end_publish(&room, *track, publish_done_status::GOING_AWAY, false);
                }
            }
            purpose => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, purpose, "GOAWAY");
            }
        }
    }

    /// A request of the gatherer is over, and was let go of.
    ///
    /// WHY the `SUBSCRIBE_NAMESPACE` is asked for again alone, after a wait
    /// that grows ([`Pause::SERVING`]), and the session goes on: what the session
    /// does for the participants and the rooms is not over with it. A relay
    /// that ends it every time (one that tells more namespaces than the
    /// session keeps, `TooManyNamespaces`) is asked no more often than every
    /// 30 seconds. What it misses: a `NAMESPACE_DONE` that comes while no
    /// request is open; that participant stays followed, and is asked again
    /// for its `said` as many times as `kept::decide` lets it.
    fn request_over(&mut self, token: u64, purpose: &Purpose, why: &str) {
        match purpose {
            Purpose::Watch(index) => {
                if let Some(watch) = self.watches.get_mut(*index) {
                    let wait = watch.ended(Instant::now());
                    watch_ended(watch.prefix(), why, wait);
                }
            }
            Purpose::Person(person, track) => {
                self.track_over(person, *track, Happened::NotSent, why);
            }
            Purpose::Room(room, track) => {
                let room = room.clone();
                println!(
                    "{}_NOT_PUBLISHED room={room:?} why={why:?}",
                    room_word(*track),
                    room = room.to_string()
                );
                self.unpublish(&room, *track, token, true);
                self.ending.remove(&token);
            }
        }
    }

    /// The track of `request` is malformed, and the session ended the
    /// request: §2.4.2, "it MUST cancel any corresponding subscription or
    /// fetches for that Track". Its stream is reset with `MALFORMED_TRACK`.
    fn malformed(&mut self, request: RequestId) {
        let token = self.own.token_of(request);
        let Some(token) = token else {
            return;
        };
        let Some(purpose) = self.let_go(token, Command::Cancel(stream_reset_code::MALFORMED_TRACK))
        else {
            return;
        };
        match &purpose {
            Purpose::Person(person, track) => {
                let end = Happened::Ended;
                self.track_over(person, *track, end, "the track is malformed");
            }
            purpose => self.request_over(token, purpose, "the track is malformed"),
        }
    }

    /// The `PUBLISH_OK` of a room's track: what waited for it is sent.
    fn room_published(
        &mut self,
        token: u64,
        room: &Room,
        track: RoomTrack,
        forward: bool,
    ) -> Result<()> {
        let waiting = self
            .published
            .get_mut(&(room.clone(), track))
            .map(|entry| entry.established(token, forward))
            .unwrap_or_default();
        let shown = room.to_string();
        let word = room_word(track);
        match track {
            RoomTrack::Lines => {
                let largest = self.gatherer.rooms.largest(room);
                let largest = largest.map_or_else(|| "none".to_string(), |at| at.to_string());
                println!("{word}_PUBLISHED room={shown:?} forward={forward} largest={largest}");
            }
            RoomTrack::Reactions | RoomTrack::Present => {
                println!("{word}_PUBLISHED room={shown:?} forward={forward}");
            }
        }
        for (group_id, payload) in waiting {
            self.send_live(room, track, group_id, payload)?;
        }
        Ok(())
    }

    /// The meaning of a message on a request stream of the gatherer.
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
            (Purpose::Watch(index), RequestEvent::NamespacesSubscribed) => {
                if let Some(watch) = self.watches.get_mut(*index) {
                    watch.subscribed();
                    println!("WATCHING prefix={}", shown(watch.prefix()));
                }
            }
            (Purpose::Watch(_), RequestEvent::Namespace(namespace)) => {
                self.on_namespace(&namespace)?;
            }
            (Purpose::Watch(_), RequestEvent::NamespaceDone(namespace)) => {
                self.on_namespace_done(&namespace)?;
            }
            (Purpose::Person(person, track), RequestEvent::SubscribeEstablished { .. }) => {
                self.track_established(person, *track)?;
            }
            (Purpose::Room(room, track), RequestEvent::PublishEstablished { forward, .. }) => {
                let (room, track) = (room.clone(), *track);
                self.room_published(token, &room, track, forward)?;
            }
            (Purpose::Room(..), RequestEvent::SubscriptionUpdated { response, .. }) => {
                self.tell(token, Command::Say(response));
            }
            // §10.9.1: the update is refused, and the subscription ends with
            // UPDATE_FAILED.
            // The PUBLISH_DONE goes once the streams of the room being sent
            // are closed (§10.11), and the room is published again then.
            (Purpose::Room(room, track), RequestEvent::UpdateRefused { response }) => {
                let (room, track) = (room.clone(), *track);
                self.tell(token, Command::Say(response));
                if self
                    .published
                    .get(&(room.clone(), track))
                    .is_some_and(|entry| entry.publish() == Some(token))
                {
                    self.end_publish(&room, track, publish_done_status::UPDATE_FAILED, false);
                }
            }
            (Purpose::Person(person, track), RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                let end = Happened::Refused {
                    error_code: error.error_code,
                    named_ms: error.retry_wait_ms(),
                };
                self.track_over(person, *track, end, &why);
            }
            (Purpose::Watch(index), RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                if let Some(watch) = self.watches.get_mut(*index) {
                    let wait = watch.refused(error.retry_wait_ms(), Instant::now());
                    watch_ended(watch.prefix(), &why, wait);
                }
            }
            (_, RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                self.request_over(token, purpose, &why);
            }
            (Purpose::Person(person, track), RequestEvent::SubscriptionDone(done)) => {
                println!(
                    "{}SUBSCRIPTION_DONE room={:?} name={:?} status={:#x}",
                    person_word(*track),
                    person.room.to_string(),
                    person.name,
                    done.status_code
                );
                self.let_go(token, Command::Finish);
                self.track_over(person, *track, Happened::Ended, "PUBLISH_DONE");
            }
            (
                _,
                RequestEvent::TrackNotProcessed {
                    refusal: TrackRefusal::Malformed,
                    ..
                },
            ) => {
                let request = self.own.get(token).map(|own| own.request);
                if let Some(request) = request {
                    self.malformed(request);
                }
            }
            (
                _,
                RequestEvent::TrackNotProcessed { .. } | RequestEvent::TooManyNamespaces { .. },
            ) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, purpose, "the session could not take its answer");
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

    /// A FETCH of the relay: for a range of the `lines` of a room, which is
    /// answered from the store ([`xmoq_chat::fetch`]).
    fn on_fetch(&mut self, asked: AskedFetch<Send>) -> Result<()> {
        // WHY a Joining FETCH is refused: the gatherer has no subscriber of
        // its own to join; the relay turns a viewer's Joining FETCH into a
        // Standalone one before it asks a publisher (`Relay::join` of
        // `crates/relay-core/src/fetch.rs`).
        let FetchRange::Standalone {
            full_track_name,
            start,
            end,
        } = asked.fetch.range.clone()
        else {
            return self.refuse(asked, request_error_code::NOT_SUPPORTED);
        };
        let room = match track_of(&full_track_name) {
            Some(ChatTrack::OfRoom(room, RoomTrack::Lines)) if self.taken.has(&room) => room,
            // WHY refused: nothing of the counts is kept (decided on
            // 2026-10-26), and the next Object says the state anew.
            Some(ChatTrack::OfRoom(room, RoomTrack::Reactions | RoomTrack::Present))
                if self.taken.has(&room) =>
            {
                return self.refuse(asked, request_error_code::NOT_SUPPORTED);
            }
            _ => return self.refuse(asked, request_error_code::DOES_NOT_EXIST),
        };
        if self.requests.full() {
            return self.refuse(asked, request_error_code::EXCESSIVE_LOAD);
        }
        let group_order = answers::group_order(&asked.fetch);
        let largest = if self.gatherer.rooms.knows(&room) {
            self.gatherer.rooms.largest(&room)
        } else {
            self.store.next_number(&room)?.checked_sub(1)
        };
        let most = self.gatherer.limits().fetch_lines;
        let planned = match plan(start, end, largest, most, group_order) {
            Ok(planned) => planned,
            Err(code) => {
                println!(
                    "FETCH_REFUSED room={room:?} error_code={code:#x}",
                    room = room.to_string()
                );
                return self.refuse(asked, code);
            }
        };
        let lines = match planned.groups {
            Some(range) => self.store.lines(&room, range, self.requests.room()),
            None => Ok(Some(Vec::new())),
        };
        let mut lines = match lines {
            Ok(Some(lines)) => lines,
            // §10.6.2: "EXCESSIVE_LOAD: The responder is overloaded and
            // cannot process the request at this time" (`MAX_BYTES_HELD`).
            Ok(None) => {
                println!(
                    "FETCH_REFUSED room={room:?} error_code=0x9 why=too-many-bytes-held",
                    room = room.to_string()
                );
                return self.refuse(asked, request_error_code::EXCESSIVE_LOAD);
            }
            Err(error) => {
                println!(
                    "FETCH_REFUSED room={room:?} error_code=0x0 store_error={error:#}",
                    room = room.to_string()
                );
                return self.refuse(asked, request_error_code::INTERNAL_ERROR);
            }
        };
        if group_order == GroupOrder::Descending {
            lines.reverse();
        }
        let count = lines.len();
        let pieces = pieces_of(
            lines.iter().map(|line| (line.number, line.payload())),
            planned.left_out,
        );
        drop(lines);
        // The store gave no more than there is room for.
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
            "FETCH_ANSWERED room={room:?} start={},{} end={},{} lines={count} unknown_through={}",
            start.group,
            start.object,
            planned.end_location.group,
            planned.end_location.object,
            planned.left_out.map_or_else(
                || "none".to_string(),
                |(location, _)| location.group.to_string()
            ),
            room = room.to_string()
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
