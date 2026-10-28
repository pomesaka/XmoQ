//! One `MoQ` Transport session of the pet with the relay: what it asks for,
//! what it announces and publishes, and the FETCH requests it answers.
//!
//! The pet is an actor of every room's garden, named `pet`
//! (`fixtures/pet-tracks/README.md`). It is told of the rooms by one
//! `SUBSCRIBE_NAMESPACE` for the prefix, as the chat's gatherer is
//! (`crates/chat/src/bin/xmoq-chat/session.rs`, where the reasons are): it
//! learns of a room by the namespaces of the persons in it,
//! `xmoq/pet/<room>/<name>` (`person_of` of `src/tracks.rs`, why there), as
//! the garden does. For each room it subscribes to `garden`, with a Joining FETCH of its latest Group
//! (§10.12.2) as the transcriber fetches a catalog, announces
//! `xmoq/pet/<room>/pet` with a `PUBLISH_NAMESPACE`, so that the garden and
//! the pages are told of the pet as of any actor, and publishes `state` and
//! `acted` in it with a PUBLISH each.
//!
//! WHY PUBLISH, and not the answer to the relay's SUBSCRIBE that the
//! announcement would bring: the publishing of a track of one Object a
//! Group (`xmoq-publisher`) is a PUBLISH, as the gatherer's, and the relay
//! asks no SUBSCRIBE of a session that is a source of the track already
//! (`ask_the_publishers` of `crates/relay-core/src/routing.rs`). One it asks
//! before the `PUBLISH_OK` is refused with `NOT_SUPPORTED`
//! (`PeerRequests::on_event` of `xmoq-publisher`).

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use tokio::sync::{mpsc, watch};
use xmoq_again::next_session::{Pause, new_url};
use xmoq_moqt::data::ObjectStatus;
use xmoq_moqt::early::{Arrived, EarlyObjects, held_bytes};
use xmoq_moqt::fetch::{FetchEvent, FetchHeader, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::immutable::TrackRefusal;
use xmoq_moqt::message::{FetchRange, publish_done_status, request_error_code};
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::{GroupOrder, Parameter, SubscriptionFilter};
use xmoq_moqt::session::{
    ControlEvent, FetchStreamOpened, MalformedRequest, RequestEvent, RequestId, Role, Session,
    SubscribedAlias, Transport,
};
use xmoq_moqt::{RequestFault, stream_reset_code};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchBody, FetchChunk, PeerRequestStreamEvent, RequestClose, RequestReadError,
    StreamConfig, StreamEnd, StreamStops, UniEvent, client, read_request_streams, read_uni_streams,
};
use xmoq_pet::args::AskAgain;
use xmoq_pet::decide::{Decision, Upon, table};
use xmoq_pet::perceive::Happening;
use xmoq_pet::pet::Arrival;
use xmoq_pet::rooms::{Joined, Left, MAX_ROOMS, Rooms};
use xmoq_pet::tracks::{self, Mine};
use xmoq_pet_tracks::names::{GardenTrack, PREFIX};
use xmoq_pet_tracks::payloads::{ActionKind, read_garden};
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

use crate::decisions::{Answered, Decider};

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

/// What the tasks of the pet's request streams said and the loop did not
/// take yet; they wait when it is full. Provisional.
const HEARD_QUEUE: usize = 256;

/// The most bytes the pet holds of one Object of `garden` before it has
/// decoded it, on a subgroup stream and on the stream of the Joining FETCH;
/// a stream with more is stopped, and that garden is not perceived. WHY
/// 128 KiB: a garden is 8 things and 16 events at the most, each with the
/// name of an actor, which is a field of a namespace and can be nearly the
/// 4,096 bytes of a Full Track Name (§2.4.1), and a talk of 120 bytes that
/// JSON can write six times as long; about 112 KB. **Provisional.**
const GARDEN_MOST: usize = 128 * 1024;

/// The states of a room that wait for the `PUBLISH_OK` of `state`, the
/// oldest dropped first. WHY 2: each says the whole pet, which the next
/// says anew. **Provisional.**
const STATES_WAITING: usize = 2;

/// The actions of a room that wait for the `PUBLISH_OK` of `acted`, the
/// oldest dropped first. WHY more than the states: each is one thing done
/// to the garden, which no later one says again; the pet does one in
/// several seconds at the most. **Provisional.**
const ACTIONS_WAITING: usize = 4;

/// Subgroup streams of Groups being written at once. Past it a Group is
/// not sent live; for a state, the next says it anew, at the latest
/// [`STATE_AGAIN_MS`] after. **Provisional.**
///
/// [`STATE_AGAIN_MS`]: xmoq_pet::rooms::STATE_AGAIN_MS
const GROUPS_BEING_SENT: usize = 256;

/// The most bytes of payload the answers to FETCH requests being written
/// hold together. WHY 1 MiB: an answer is one state of under 400 bytes, and
/// [`MAX_ANSWERED_AT_ONCE`] are answered at once. **Provisional.**
const MAX_BYTES_HELD: usize = 1024 * 1024;

/// An Object of a track of the pet that waits for its `PUBLISH_OK`: its
/// Group ID and its payload.
type Waiting = (u64, Vec<u8>);

/// A track the pet publishes: its room, and which.
type Key = (String, Mine);

/// A garden read from the stream of a Joining FETCH: its room, the
/// request, and its Group and payload where the stream had one.
type Fetched = (String, RequestId, Option<(u64, Vec<u8>)>);

/// How a session ended without an error.
#[derive(Debug)]
pub enum SessionEnd {
    /// The pet was told to stop.
    Stopped,
    /// The relay sent GOAWAY on the control stream (§10.4), with the URL
    /// to go to, or none for the same.
    Moved(Option<String>),
}

/// What of a room the pet asks for again after it is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Asked {
    /// The `PUBLISH_NAMESPACE` of the pet's namespace.
    Announce,
    /// The SUBSCRIBE to `garden`.
    Garden,
}

impl Asked {
    fn word(self) -> &'static str {
        match self {
            Self::Announce => "announce",
            Self::Garden => "garden",
        }
    }
}

/// The requests of a room the pet is in on this session.
struct Here {
    /// The token of its `PUBLISH_NAMESPACE`, while it has one.
    announce: Option<u64>,
    /// Whether that was answered with `REQUEST_OK`.
    announced: bool,
    announce_tries: Tries,
    /// The token of the SUBSCRIBE to `garden`, while it has one.
    garden: Option<u64>,
    garden_tries: Tries,
    /// The token of the Joining FETCH of `garden`, while it has one.
    fetch: Option<u64>,
}

/// What a request of the pet is for.
#[derive(Debug, Clone)]
enum Purpose {
    /// The `SUBSCRIBE_NAMESPACE` for `xmoq/pet`.
    Watch,
    /// The `PUBLISH_NAMESPACE` of `xmoq/pet/<room>/pet`.
    Announce(String),
    /// The SUBSCRIBE to the `garden` of a room.
    Garden(String),
    /// The Joining FETCH of the `garden` of a room: the garden as it is.
    GardenFetch(String),
    /// The PUBLISH of a track of the pet in a room.
    Publish(String, Mine),
}

/// What decided a batch, for the log.
enum Source {
    Api,
    Table(&'static str),
}

/// Everything of one session.
struct Tending<'a> {
    connection: &'a Arc<WtConnection>,
    rooms: &'a mut Rooms,
    decider: Option<&'a mut Decider>,
    session: Session,
    stops: StreamStops,
    own: OwnRequests<Purpose>,
    /// The `state` and `acted` of each room the pet is in, with the Objects
    /// that wait for their `PUBLISH_OK`.
    publishing: Publishing<Key, Waiting>,
    /// The rooms the pet is in on this session.
    here: HashMap<String, Here>,
    /// What is to be asked for again, and when.
    asking_again: HashMap<(String, Asked), Instant>,
    ask_again: AskAgain,
    /// Objects of `garden` read before the `SUBSCRIBE_OK` that gives their
    /// Track Alias (`xmoq_moqt::early`): the alias, the Group, the payload.
    early: EarlyObjects<(u64, u64, Vec<u8>)>,
    /// The room each Joining FETCH of `garden` is for, by its Request ID.
    garden_fetches: HashMap<u64, String>,
    /// Where the tasks that read the stream of such a FETCH put the garden.
    fetched: mpsc::Sender<Fetched>,
    /// The FETCH requests of the relay being answered, and those refused.
    requests: PeerRequests,
    watch: KeptNamespace,
    /// `--auth-key-hex`.
    key: Option<&'a RoomKey>,
    began: Instant,
}

/// The pet's clock since the epoch, in milliseconds: the times of `state`.
fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| u64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

/// The seed of a new pet's whims. WHY `RandomState`: `std` keys it anew for
/// each process, so pets of two runs differ, with no crate for it.
fn seed(room: &str, now: u64) -> u64 {
    std::hash::RandomState::new().hash_one((room, now))
}

/// What the session says the peer did wrong, as an error that carries its
/// code (`xmoq_moqt_io::client::termination_code`).
fn fault(fault: RequestFault) -> anyhow::Error {
    match fault {
        RequestFault::Peer(error) => error.into(),
        RequestFault::Usage(error) => anyhow!("the pet used its session wrongly: {error}"),
    }
}

/// Reads the stream of a Joining FETCH of `garden` to its end: the last
/// Object, which is the garden as it is, with its Group.
async fn last_garden(header: FetchHeader, mut body: FetchBody) -> Option<(u64, Vec<u8>)> {
    let mut decoder = FetchStreamDecoder::new(GARDEN_MOST, GroupOrder::Ascending);
    // The readers took the header off; the decoder wants to see one.
    decoder
        .push(&FetchStreamEncoder::new(header, GroupOrder::Ascending).take_bytes())
        .ok()?;
    let mut latest = None;
    loop {
        while let Some(event) = decoder.next_event().ok()? {
            if let FetchEvent::Object(object) = event {
                latest = Some((object.group_id, object.payload));
            }
        }
        match body.next().await {
            FetchChunk::Bytes(bytes) => decoder.push(&bytes).ok()?,
            FetchChunk::End(_) | FetchChunk::Stopped => return latest,
        }
    }
}

/// The thing an action of the pet is done to.
fn thing_of(action: &ActionKind) -> Option<u64> {
    match action {
        ActionKind::Eat { thing } | ActionKind::Take { thing } => Some(*thing),
        ActionKind::Pet
        | ActionKind::Feed { .. }
        | ActionKind::Call
        | ActionKind::Throw { .. }
        | ActionKind::Talk { .. } => None,
    }
}

/// Runs one session on `connection` until it ends.
#[expect(
    clippy::too_many_lines,
    reason = "the readers, the state and the one loop of the session, which share their queues"
)]
pub async fn tend(
    connection: &Arc<WtConnection>,
    kept: &mut client::UntilClosed<WtConnection>,
    (rooms, decider, answered): (
        &mut Rooms,
        Option<&mut Decider>,
        &mut mpsc::Receiver<Answered>,
    ),
    key: Option<&RoomKey>,
    ask_again: AskAgain,
    stop: &mut watch::Receiver<bool>,
) -> Result<SessionEnd> {
    let stops = StreamStops::default();
    let (uni_queue, mut uni) = mpsc::channel(UNI_QUEUE);
    let stream_config = StreamConfig {
        subgroup_buffer_limit: GARDEN_MOST,
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
    let (drained, mut drained_tracks) = mpsc::channel(PEER_REQUEST_QUEUE);
    // WHY as long as the rooms: one Joining FETCH a room at the most.
    let (fetched, mut fetched_by_loop) = mpsc::channel(MAX_ROOMS);
    let mut tending = Tending {
        connection,
        rooms,
        decider,
        session: Session::new(Role::Client, Transport::WebTransport),
        stops,
        own: OwnRequests::new(Arc::clone(connection), heard, OPEN_WAIT),
        publishing: Publishing::new(drained, GROUPS_BEING_SENT),
        here: HashMap::new(),
        asking_again: HashMap::new(),
        ask_again,
        early: EarlyObjects::default(),
        garden_fetches: HashMap::new(),
        fetched,
        requests: PeerRequests::new(Bounds {
            at_once: MAX_ANSWERED_AT_ONCE,
            bytes: MAX_BYTES_HELD,
            say_queue: COMMAND_QUEUE,
        }),
        watch: KeptNamespace::new(tracks::prefix()?, Pause::SERVING),
        key,
        began: Instant::now(),
    };

    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    // The relay looks at no namespace or role of the token of a SETUP.
    let token = tending.sub_token();
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
    // Which rooms there are is what this session is told; the pets stay.
    tending.rooms.leave_everybody(wall_ms());
    tending.watch()?;

    let ended = loop {
        // WHY taken before the select: its arms borrow parts of the state
        // mutably.
        let (next_watch, next_asked, next_pet) = (
            tending.watch.again_at(),
            tending.asking_again.values().min().copied(),
            tending.next_pet(),
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
            Some((room, request, read)) = fetched_by_loop.recv() => {
                tending.session.on_fetch_stream_closed(request);
                tending.garden_fetched(&room);
                if let Some((group, payload)) = read {
                    tending.on_garden(&room, group, &payload);
                }
            }
            note = tending.requests.next_note() => tending.fetch_noted(note),
            Some(token) = drained_tracks.recv() => tending.publish_done(token)?,
            () = sleep_until(next_watch) => {
                if tending.watch.due(Instant::now()) {
                    tending.watch()?;
                }
            }
            () = sleep_until(next_asked) => tending.ask_again()?,
            () = sleep_until(next_pet) => tending.go_on()?,
            Some(answer) = answered.recv() => tending.on_decision(&answer)?,
            () = tending.publishing.sent() => {}
        }
    };
    Ok(ended)
}

impl Tending<'_> {
    /// A token of the pet's own for `role` in `namespace` (its fields joined
    /// with `/`) from now, as an AUTHORIZATION TOKEN; `None` without
    /// `--auth-key-hex`. WHY made anew for each request: as the chat's
    /// gatherer makes its own (`Gathering::token`), so that none expires on
    /// a long session.
    fn token(&self, namespace: &str, role: TokenRole) -> Option<Vec<u8>> {
        let key = self.key?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        Some(key.authorization_token(namespace, role, now))
    }

    /// A `sub` token for every namespace of the garden (`xmoq/pet/*`).
    /// WHY under the prefix: the relay takes a `SUBSCRIBE_NAMESPACE` with a
    /// token whose last field is `*` alone (`crates/relay/src/auth.rs`),
    /// and the pet follows every room.
    fn sub_token(&self) -> Option<Vec<u8>> {
        self.token(&format!("{}/*", PREFIX.join("/")), TokenRole::Sub)
    }

    /// [`Self::sub_token`] as the parameter of a request.
    fn sub_parameter(&self) -> Option<Parameter> {
        self.sub_token().map(Parameter::AuthorizationToken)
    }

    /// A `pub` token of the pet's namespace of `room` alone, as the
    /// parameter of a request. WHY of that namespace and not of the prefix:
    /// it allows the pet to publish as the actor `pet` and as no other.
    fn pub_parameter(&self, room: &str) -> Option<Parameter> {
        self.token(&tracks::mine_text(room), TokenRole::Pub)
            .map(Parameter::AuthorizationToken)
    }

    /// Asks for the namespaces under `xmoq/pet`.
    fn watch(&mut self) -> Result<()> {
        self.watch.asked(Instant::now());
        let prefix = self.watch.prefix().clone();
        let (request, message) = self
            .session
            .subscribe_namespace(prefix, self.sub_parameter().into_iter().collect())?;
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

    /// Announces the pet's namespace of `room`.
    fn announce(&mut self, room: &str) -> Result<()> {
        self.asking_again
            .remove(&(room.to_string(), Asked::Announce));
        let parameters: Vec<Parameter> = self.pub_parameter(room).into_iter().collect();
        let (request, message) = self
            .session
            .publish_namespace(tracks::mine(room)?, parameters.clone())?;
        let token = self.own.open(
            Purpose::Announce(room.to_string()),
            request,
            message,
            &parameters,
        );
        if let Some(here) = self.here.get_mut(room) {
            here.announce = Some(token);
            here.announced = false;
        }
        Ok(())
    }

    /// Publishes the track `mine` of `room`, unless it is published or
    /// waits to be published again.
    fn publish(&mut self, room: &str, mine: Mine) -> Result<()> {
        let key = (room.to_string(), mine);
        let most = match mine {
            Mine::State => STATES_WAITING,
            Mine::Acted => ACTIONS_WAITING,
        };
        self.publishing.track(key.clone(), most);
        let largest = self.rooms.get(room).and_then(|entry| match mine {
            Mine::State => entry.latest_group(),
            Mine::Acted => entry.acted_group(),
        });
        let parameters = publishing::parameters(largest, self.pub_parameter(room));
        let published = self.publishing.publish(
            &key,
            tracks::my_track(room, mine)?,
            &parameters,
            &mut self.session,
            &mut self.own,
            Purpose::Publish(room.to_string(), mine),
        );
        if let Publish::Refused(_) = published {
            println!(
                "NOT_PUBLISHED room={room:?} track={} why=\"the PUBLISH before is ending\"",
                mine.word()
            );
        }
        Ok(())
    }

    /// Ends the PUBLISH of a track with `status`; its `PUBLISH_DONE` goes
    /// once its streams are closed ([`Self::publish_done`]). `forget`: the
    /// room is left, and its publication goes.
    fn end_publish(&mut self, key: &Key, status: u64, forget: bool) {
        self.publishing.end(key, status);
        if forget {
            self.publishing.forget(key);
        }
    }

    /// Every stream of an ending PUBLISH is closed: its `PUBLISH_DONE`
    /// (§10.11) and a FIN. Where the pet is in the room still, it is
    /// published anew.
    fn publish_done(&mut self, token: u64) -> Result<()> {
        let Some(done) = self.publishing.done(token, &mut self.session, &self.own)? else {
            return Ok(());
        };
        let (room, mine) = done.key;
        if let Some(message) = done.message {
            self.tell(token, Command::Say(message));
            self.let_go(token, Command::Finish);
            println!(
                "PUBLISH_DONE room={room:?} track={} status={:#x} streams={}",
                mine.word(),
                done.status,
                done.streams
            );
        }
        if self.here.contains_key(&room) {
            let key = (room.clone(), mine);
            if let Some(entry) = self.publishing.get_mut(&key) {
                entry.may_publish_again();
            }
            self.publish(&room, mine)?;
        }
        Ok(())
    }

    /// The PUBLISH of a track is over; the next Object publishes it again.
    fn unpublish(&mut self, key: &Key, token: u64) {
        if let Some(entry) = self.publishing.get_mut(key)
            && entry.unpublished(token, true, Instant::now())
        {
            println!("UNPUBLISHED room={:?} track={}", key.0, key.1.word());
        }
    }

    /// Subscribes to the `garden` of `room`, with the Largest Object filter
    /// (§5.1.2); the garden as it is comes by the Joining FETCH that follows
    /// its `SUBSCRIBE_OK` ([`Self::garden_established`]).
    fn subscribe_garden(&mut self, room: &str) -> Result<()> {
        let mut parameters = vec![
            Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
            Parameter::Forward(true),
            Parameter::SubscriberPriority(128),
            Parameter::GroupOrder(GroupOrder::Ascending),
        ];
        parameters.extend(self.sub_parameter());
        self.asking_again.remove(&(room.to_string(), Asked::Garden));
        let Ok((request, message)) = self
            .session
            .subscribe(tracks::garden(room)?, parameters.clone())
        else {
            println!("NOT_SUBSCRIBED room={room:?} why=\"a subscription is there already\"");
            return Ok(());
        };
        let token = self.own.open(
            Purpose::Garden(room.to_string()),
            request,
            message,
            &parameters,
        );
        if let Some(here) = self.here.get_mut(room) {
            here.garden = Some(token);
        }
        Ok(())
    }

    /// The `garden` of a room was answered: the garden as it is, the latest
    /// Group, is fetched (§10.12.2: a Joining FETCH 0 Groups before the
    /// subscription's start).
    fn garden_established(&mut self, token: u64, room: &str) -> Result<()> {
        println!("SUBSCRIBED room={room:?}");
        let keeping = self.ask_again.keeping();
        if let Some(here) = self.here.get_mut(room) {
            decide(Happened::Subscribed, keeping, &mut here.garden_tries);
        }
        if let Some(entry) = self.rooms.get_mut(room) {
            entry.perceiver.subscribed();
        }
        self.settle_early();
        let Some(own) = self.own.get(token) else {
            return Ok(());
        };
        let range = FetchRange::RelativeJoining {
            joining_request_id: own.request.value(),
            joining_start: 0,
        };
        let parameters: Vec<Parameter> = self.sub_parameter().into_iter().collect();
        let (request, message) = self.session.fetch(range, parameters.clone())?;
        self.garden_fetches
            .insert(request.value(), room.to_string());
        let fetch = self.own.open(
            Purpose::GardenFetch(room.to_string()),
            request,
            message,
            &parameters,
        );
        if let Some(here) = self.here.get_mut(room) {
            here.fetch = Some(fetch);
        }
        Ok(())
    }

    /// The stream of a room's Joining FETCH was read: its request is
    /// finished, and the relay's direction is waited for. WHY: a request
    /// left open for each room stays in the session, which holds 256 at the
    /// most.
    fn garden_fetched(&mut self, room: &str) {
        self.garden_fetches.retain(|_, fetched| fetched != room);
        let token = self.here.get_mut(room).and_then(|here| here.fetch.take());
        if let Some(token) = token {
            self.let_go(token, Command::Finish);
        }
    }

    /// The announcement or the `garden` of a room is over: it is asked for
    /// again after a pause, or not until the room's NAMESPACE comes again
    /// (`kept::decide` of `xmoq-subscriber`).
    fn kept_over(&mut self, room: &str, asked: Asked, end: Happened, why: &str) {
        let what = asked.word();
        println!("NOT_KEPT room={room:?} what={what} why={why:?}");
        let keeping = self.ask_again.keeping();
        let Some(here) = self.here.get_mut(room) else {
            return;
        };
        let tries = match asked {
            Asked::Announce => {
                here.announce = None;
                here.announced = false;
                &mut here.announce_tries
            }
            Asked::Garden => {
                here.garden = None;
                &mut here.garden_tries
            }
        };
        match decide(end, keeping, tries) {
            Becomes::Again(pause) => {
                println!("ASKING_AGAIN room={room:?} what={what} in_ms={pause}");
                self.asking_again.insert(
                    (room.to_string(), asked),
                    Instant::now() + Duration::from_millis(pause),
                );
            }
            Becomes::GivenUp | Becomes::Subscribed => {
                println!("NOT_ASKING_AGAIN room={room:?} what={what}");
            }
        }
    }

    /// The Objects held for an alias the session knows now are taken, and
    /// the streams of those no SUBSCRIBE can give any more are stopped.
    fn settle_early(&mut self) {
        if self.early.is_empty() {
            return;
        }
        let settled = self.early.settle(&self.session);
        for (stream, code) in settled.stopped {
            self.stops.stop(stream, code);
        }
        for (track_alias, group, payload) in settled.released {
            self.on_object(track_alias, group, &payload);
        }
    }

    /// Asks again for what is due: an announcement, or a `garden` of a room
    /// somebody is in.
    fn ask_again(&mut self) -> Result<()> {
        let now = Instant::now();
        let due: Vec<(String, Asked)> = self
            .asking_again
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(asked, _)| asked.clone())
            .collect();
        for (room, asked) in due {
            self.asking_again.remove(&(room.clone(), asked));
            let Some(here) = self.here.get(&room) else {
                continue;
            };
            match asked {
                Asked::Announce if here.announce.is_none() => self.announce(&room)?,
                Asked::Garden
                    if here.garden.is_none()
                        && self
                            .rooms
                            .get(&room)
                            .is_some_and(xmoq_pet::rooms::Room::has_people) =>
                {
                    self.subscribe_garden(&room)?;
                }
                Asked::Announce | Asked::Garden => {}
            }
        }
        Ok(())
    }

    /// A NAMESPACE under `xmoq/pet`: a person, where it is one, in whose
    /// room the pet is.
    fn on_namespace(&mut self, namespace: &TrackNamespace) -> Result<()> {
        let Some(person) = tracks::person_of(namespace) else {
            println!("NAMESPACE_IGNORED namespace={}", shown(namespace));
            return Ok(());
        };
        let room = person.room.clone();
        let now = wall_ms();
        let joined = self.rooms.join(&person, now, seed(&room, now));
        if matches!(joined, Joined::TooManyRooms | Joined::TooManyPeople) {
            println!(
                "PERSON_IGNORED room={room:?} name={:?} why={}",
                person.name,
                joined.word()
            );
            return Ok(());
        }
        // A NAMESPACE that comes again counts the times anew.
        let tries = self.ask_again.tries();
        let here = self.here.entry(room.clone()).or_insert_with(|| Here {
            announce: None,
            announced: false,
            announce_tries: tries.clone(),
            garden: None,
            garden_tries: tries.clone(),
            fetch: None,
        });
        here.announce_tries = tries.clone();
        here.garden_tries = tries;
        let (announce, subscribe) = (here.announce.is_none(), here.garden.is_none());
        if let Joined::Now { new_room } = joined {
            println!(
                "PERSON room={room:?} name={:?} new_room={new_room}",
                person.name
            );
        }
        if announce {
            self.announce(&room)?;
        }
        if subscribe {
            self.subscribe_garden(&room)?;
        }
        self.publish(&room, Mine::Acted)?;
        if let Joined::Now { .. } = joined {
            // One who comes gets a state at once.
            self.send_state(&room)?;
        }
        Ok(())
    }

    /// A `NAMESPACE_DONE` under `xmoq/pet`: a person is gone. Where nobody
    /// is in the room, its `garden` is let go of; the pet stays announced
    /// and published until the room is forgotten ([`Self::leave_room`]).
    fn on_namespace_done(&mut self, namespace: &TrackNamespace) {
        let Some(person) = tracks::person_of(namespace) else {
            return;
        };
        let room = person.room.clone();
        let left = self.rooms.leave(&person, wall_ms());
        if left != Left::NotThere {
            println!("PERSON_GONE room={room:?} name={:?}", person.name);
        }
        if left != Left::RoomEmpty {
            return;
        }
        self.asking_again.remove(&(room.clone(), Asked::Garden));
        let tokens = self
            .here
            .get_mut(&room)
            .map(|here| [here.garden.take(), here.fetch.take()]);
        for token in tokens.into_iter().flatten().flatten() {
            self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
        }
        self.garden_fetches.retain(|_, fetched| *fetched != room);
        println!("ROOM_EMPTY room={room:?}");
    }

    /// The room is forgotten: the pet's namespace is withdrawn (§6.2: its
    /// request is cancelled), its tracks end with `TRACK_ENDED`, and what
    /// it asked of the room is let go of.
    fn leave_room(&mut self, room: &str) {
        self.asking_again
            .retain(|(asked_room, _), _| asked_room != room);
        self.garden_fetches.retain(|_, fetched| fetched != room);
        if let Some(here) = self.here.remove(room) {
            for token in [here.announce, here.garden, here.fetch]
                .into_iter()
                .flatten()
            {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
            }
        }
        for mine in [Mine::State, Mine::Acted] {
            self.end_publish(
                &(room.to_string(), mine),
                publish_done_status::TRACK_ENDED,
                true,
            );
        }
    }

    /// A garden of `room`, Group `group`: what is new of it goes into the
    /// room's batch.
    fn on_garden(&mut self, room: &str, group: u64, payload: &[u8]) {
        let garden = match read_garden(payload) {
            Ok(garden) => garden,
            Err(error) => {
                println!(
                    "GARDEN_DROPPED room={room:?} group={group} why={}",
                    error.word()
                );
                return;
            }
        };
        let Some(entry) = self.rooms.get_mut(room) else {
            return;
        };
        let Some(perceived) = entry.perceiver.read(group, garden) else {
            return;
        };
        let now = wall_ms();
        let events = perceived
            .new
            .iter()
            .filter(|happening| matches!(happening, Happening::Done(_)))
            .count();
        let things = perceived.new.len() - events;
        let mut dropped = 0;
        for happening in perceived.new {
            if entry.batch.add(happening, now).is_err() {
                dropped += 1;
            }
        }
        // WHY not for every garden: the garden is sent again without a
        // change every few seconds.
        if events + things + perceived.gone.len() > 0 {
            println!(
                "PERCEIVED room={room:?} group={group} events={events} things={things} gone={} floor={}",
                perceived.gone.len(),
                entry.perceiver.floor().len()
            );
        }
        if dropped > 0 {
            println!("PERCEIVED_DROPPED room={room:?} happenings={dropped} why=too-many");
        }
    }

    /// An Object of a track the pet subscribed to under `track_alias`.
    fn on_object(&mut self, track_alias: u64, group: u64, payload: &[u8]) {
        let track = match self.session.subscribed_alias(track_alias) {
            SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => {
                tracks::track_of(track)
            }
            SubscribedAlias::Unknown | SubscribedAlias::Retired => None,
        };
        // The pet subscribes to `garden` alone.
        if let Some(GardenTrack::Garden { room }) = track {
            self.on_garden(&room, group, payload);
        }
    }

    /// Sends `object` on the track `mine` of `room`, as a Group of its own,
    /// or has it wait for its `PUBLISH_OK`.
    fn send(&mut self, room: &str, mine: Mine, object: Waiting) -> Result<()> {
        let key = (room.to_string(), mine);
        if !self.publishing.live(&key) {
            self.publish(room, mine)?;
            self.publishing.wait(&key, object);
            return Ok(());
        }
        let group_id = object.0;
        let shown = room.to_string();
        let failed = move |why: String| {
            println!(
                "NOT_SENT room={shown:?} track={} group={group_id} why={why:?}",
                mine.word()
            );
        };
        if self.publishing.send(&key, self.connection, object, failed) == Sent::TooMany {
            println!(
                "NOT_SENT room={room:?} track={} group={group_id} why=too-many-at-once",
                mine.word()
            );
        }
        Ok(())
    }

    /// Sends the state of a room as it is now, where the pet is in it on
    /// this session.
    fn send_state(&mut self, room: &str) -> Result<()> {
        if !self.here.contains_key(room) {
            return Ok(());
        }
        let Some(entry) = self.rooms.get_mut(room) else {
            return Ok(());
        };
        let object = entry.write(wall_ms());
        self.send(room, Mine::State, object)
    }

    /// The pet of a room arrived where it went for a thing: what it does to
    /// it is sent on `acted`, where the thing is there still.
    fn arrived(&mut self, room: &str, arrival: Arrival) -> Result<()> {
        let kind = arrival.action.word();
        let thing = thing_of(&arrival.action).map_or_else(|| "-".to_string(), |id| id.to_string());
        if !arrival.there {
            println!("ACTED_NOT_SENT room={room:?} kind={kind} thing={thing} why=gone");
            return Ok(());
        }
        let Some(entry) = self.rooms.get_mut(room) else {
            return Ok(());
        };
        let object = entry.write_acted(arrival.action, wall_ms());
        println!(
            "ACTED_SENT room={room:?} kind={kind} thing={thing} group={}",
            object.0
        );
        self.send(room, Mine::Acted, object)
    }

    /// When a pet next has something to do.
    fn next_pet(&self) -> Option<Instant> {
        let at = self.rooms.next_at()?;
        let wait = at.saturating_sub(wall_ms());
        Some(Instant::now() + Duration::from_millis(wait))
    }

    /// What is due of every room: its batch decided, its pet arriving,
    /// looking at the floor or going on, its state published again, or the
    /// room forgotten.
    fn go_on(&mut self) -> Result<()> {
        let now = wall_ms();
        for name in self.rooms.names() {
            let Some(room) = self.rooms.get_mut(&name) else {
                continue;
            };
            if let Some(batch) = room.batch.take(now) {
                self.decide(&name, &batch, now)?;
                continue;
            }
            let arrival = room.pet.arrive(now, room.perceiver.floor());
            // WHY not while happenings are gathered: the batch is decided
            // within `BATCH_MS`, upon the floor as well.
            let looks = room.pet.look(now, room.perceiver.floor()) && room.batch.due_at().is_none();
            let changed = !looks && (room.pet.go_on(now) || arrival.is_some());
            let again = room.again_at().is_some_and(|at| at <= now);
            let garden = room.has_people();
            if let Some(arrival) = arrival {
                self.arrived(&name, arrival)?;
            }
            if looks {
                self.decide(&name, &[], now)?;
            } else if garden && (changed || again) {
                self.send_state(&name)?;
            }
        }
        for gone in self.rooms.forget(now) {
            println!("ROOM_FORGOTTEN room={gone:?}");
            self.leave_room(&gone);
        }
        Ok(())
    }

    /// A batch of a room is due, or, for an empty one, its pet looks at the
    /// floor: the API is asked where it is given and the rate lets it; the
    /// table decides otherwise.
    fn decide(&mut self, room: &str, batch: &[Happening], now: u64) -> Result<()> {
        let ms = u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX);
        let by = batch.last().map(|happening| happening.by().to_string());
        if batch.is_empty() {
            let floor = self.rooms.get(room).map_or(0, |entry| {
                entry.pet.untouched(entry.perceiver.floor()).len()
            });
            println!("LOOKING room={room:?} floor={floor}");
        }
        let why = match (self.decider.as_deref_mut(), self.rooms.get_mut(room)) {
            (Some(decider), Some(entry)) => {
                let pet = (entry.pet.state(), entry.pet.place_at(now));
                let floor = entry.pet.untouched(entry.perceiver.floor());
                let count = (batch.len(), floor.len());
                if decider.ask(room.to_string(), (floor, batch.to_vec()), pet, ms) {
                    entry.batch.asking();
                    entry.pet.think(by.as_deref(), now);
                    let garden = entry.has_people();
                    println!(
                        "THINKING room={room:?} happenings={} floor={}",
                        count.0, count.1
                    );
                    if garden {
                        self.send_state(room)?;
                    }
                    return Ok(());
                }
                "rate"
            }
            (None, _) => "no-api",
            (_, None) => return Ok(()),
        };
        let decision = self.by_table(room, batch);
        self.decided(room, batch, decision, &Source::Table(why))
    }

    /// The decision of the table for `batch` of a room, upon the floor for
    /// an empty one.
    fn by_table(&self, room: &str, batch: &[Happening]) -> Option<Decision> {
        let entry = self.rooms.get(room)?;
        table(batch, &entry.pet.untouched(entry.perceiver.floor()))
    }

    /// The API answered for the batch of a room.
    fn on_decision(&mut self, answer: &Answered) -> Result<()> {
        let Some(room) = self.rooms.get_mut(&answer.room) else {
            return Ok(());
        };
        room.batch.answered();
        match answer.decided {
            Ok(decision) => self.decided(&answer.room, &answer.batch, Some(decision), &Source::Api),
            Err(failed) => {
                let decision = self.by_table(&answer.room, &answer.batch);
                self.decided(
                    &answer.room,
                    &answer.batch,
                    decision,
                    &Source::Table(failed.word()),
                )
            }
        }
    }

    /// The pet of a room reacts as `decision` says, with the things on the
    /// floor as the garden has them now; its state is published.
    fn decided(
        &mut self,
        room: &str,
        batch: &[Happening],
        decision: Option<Decision>,
        source: &Source,
    ) -> Result<()> {
        let (Some(decision), Some(entry)) = (decision, self.rooms.get_mut(room)) else {
            return Ok(());
        };
        let now = wall_ms();
        let by = batch.last().map(Happening::by);
        let upon = if batch.is_empty() {
            Upon::Floor
        } else {
            Upon::Happened
        };
        entry
            .pet
            .react(&decision, by, entry.perceiver.floor(), now, upon);
        let state = entry.pet.state();
        let (source, why) = match source {
            Source::Api => ("api", "answered"),
            Source::Table(why) => ("table", *why),
        };
        println!(
            "DECIDED room={room:?} source={source} why={why} reaction={} mood={} bubble={}",
            decision.reaction.word(),
            state.mood,
            decision.bubble.unwrap_or("none")
        );
        println!(
            "STATE room={room:?} doing={} mood={}",
            state.doing.word(),
            state.mood
        );
        if entry.has_people() {
            self.send_state(room)?;
        }
        Ok(())
    }

    /// An event of the relay's unidirectional streams.
    fn on_uni(&mut self, event: UniEvent) -> Result<Option<SessionEnd>> {
        self.settle_early();
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
                let item = (alias, header.group_id, object.payload);
                let (group, payload) =
                    match self.early.arrive(&self.session, stream, alias, bytes, item) {
                        Arrived::Deliver((_, group, payload)) => (group, payload),
                        Arrived::Held => return Ok(None),
                        Arrived::Stop { stream, code } => {
                            self.stops.stop(stream, code);
                            return Ok(None);
                        }
                    };
                self.on_object(alias, group, &payload);
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
            UniEvent::FetchOpened { stream, header } => {
                let body = self.stops.take_fetch_body(stream);
                let FetchStreamOpened::Fetch(request) =
                    self.session.on_fetch_stream(header.request_id)?
                else {
                    drop(body);
                    return Ok(None);
                };
                let (Some(room), Some(body)) =
                    (self.garden_fetches.get(&header.request_id).cloned(), body)
                else {
                    self.session.on_fetch_stream_closed(request);
                    return Ok(None);
                };
                let fetched = self.fetched.clone();
                tokio::spawn(async move {
                    let read = last_garden(header, body).await;
                    // A queue that is gone is a session that is gone.
                    let _ = fetched.send((room, request, read)).await;
                });
            }
            // Not asked for (`StreamConfig::report_objects_begun`).
            UniEvent::SubgroupOpened { .. } | UniEvent::ObjectBegun { .. } => {}
            UniEvent::Failed(error) => return Err(error.into()),
            UniEvent::ReaderPanicked => return Err(client::ReaderPanicked.into()),
        }
        Ok(None)
    }

    /// What the task of one of the pet's request streams heard.
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
            Heard::Closed(_) | Heard::Stopped(_) if matches!(purpose, Purpose::Publish(..)) => {
                let end = match &heard {
                    Heard::Closed(RequestClose::Fin) => RequestStreamEnd::Fin,
                    Heard::Stopped(_) => RequestStreamEnd::StopSending,
                    _ => RequestStreamEnd::Reset,
                };
                let stage = (match &purpose {
                    Purpose::Publish(room, mine) => self
                        .publishing
                        .get(&(room.clone(), *mine))
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
            // A publisher that finishes its direction of a FETCH it answered
            // has not ended it: the FETCH goes on until its fetch stream ends.
            Heard::Closed(RequestClose::Fin) if matches!(purpose, Purpose::GardenFetch(_)) => {
                Ok(None)
            }
            // After its `REQUEST_OK`, a FIN of the relay is no withdrawal of
            // the namespace: §6.2 withdraws one by cancelling the request,
            // and §3.3.2 cancels "by resetting or sending STOP_SENDING"
            // (`namespace_stream_ends` of `xmoq-moqt-client`, the same).
            Heard::Closed(RequestClose::Fin)
                if matches!(&purpose, Purpose::Announce(room)
                    if self.here.get(room).is_some_and(|here| here.announced)) =>
            {
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

    /// The relay sent GOAWAY on the stream of a request of the pet: the
    /// request alone is asked again, on this session (§10.4), as the
    /// gatherer does.
    ///
    /// FIXME: a New Session URI names another session for the request, and
    /// the request is asked again on this one (§10.4: "on a session at the
    /// specified URI"); the pet keeps one session.
    fn request_moved(&mut self, token: u64, purpose: &Purpose) {
        match purpose {
            Purpose::Publish(room, mine) => {
                let key = (room.clone(), *mine);
                if self.publishing.is_publish(&key, token) {
                    self.end_publish(&key, publish_done_status::GOING_AWAY, false);
                }
            }
            purpose => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, purpose, Happened::Ended, "GOAWAY");
            }
        }
    }

    /// A request of the pet is over, and was let go of. The
    /// `SUBSCRIBE_NAMESPACE` is asked for again alone, after a wait that
    /// grows, and the session goes on, as the gatherer's (why, there).
    fn request_over(&mut self, token: u64, purpose: &Purpose, end: Happened, why: &str) {
        match purpose {
            Purpose::Watch => {
                let wait = self.watch.ended(Instant::now());
                self.watch_ended(why, wait);
            }
            Purpose::Announce(room) => self.kept_over(room, Asked::Announce, end, why),
            Purpose::Garden(room) => self.kept_over(room, Asked::Garden, end, why),
            Purpose::GardenFetch(room) => {
                println!("NOT_FETCHED room={room:?} why={why:?}");
                self.garden_fetches.retain(|_, fetched| fetched != room);
                if let Some(here) = self.here.get_mut(room) {
                    here.fetch = None;
                }
            }
            Purpose::Publish(room, mine) => {
                println!(
                    "NOT_PUBLISHED room={room:?} track={} why={why:?}",
                    mine.word()
                );
                self.unpublish(&(room.clone(), *mine), token);
                self.publishing.over(token);
            }
        }
    }

    /// The track of `request` is malformed, and the session ended the
    /// request: §2.4.2, "it MUST cancel any corresponding subscription or
    /// fetches for that Track". Its stream is reset with `MALFORMED_TRACK`.
    fn malformed(&mut self, request: RequestId) {
        self.garden_fetches.remove(&request.value());
        let Some(token) = self.own.token_of(request) else {
            return;
        };
        let Some(purpose) = self.let_go(token, Command::Cancel(stream_reset_code::MALFORMED_TRACK))
        else {
            return;
        };
        match &purpose {
            Purpose::Garden(room) => {
                self.kept_over(
                    room,
                    Asked::Garden,
                    Happened::Malformed,
                    "the track is malformed",
                );
            }
            purpose => self.request_over(token, purpose, Happened::Ended, "the track is malformed"),
        }
    }

    /// The `PUBLISH_OK` of a track of the pet: what waited for it is sent.
    fn published(&mut self, token: u64, key: &Key, forward: bool) {
        let waiting = self
            .publishing
            .get_mut(key)
            .map(|entry| entry.established(token, forward))
            .unwrap_or_default();
        let (room, mine) = (key.0.clone(), key.1);
        println!(
            "PUBLISHED room={room:?} track={} forward={forward}",
            mine.word()
        );
        for (group_id, payload) in waiting {
            let shown = room.clone();
            let failed = move |why: String| {
                println!(
                    "NOT_SENT room={shown:?} track={} group={group_id} why={why:?}",
                    mine.word()
                );
            };
            if self
                .publishing
                .send(key, self.connection, (group_id, payload), failed)
                == Sent::TooMany
            {
                println!(
                    "NOT_SENT room={room:?} track={} group={group_id} why=too-many-at-once",
                    mine.word()
                );
            }
        }
    }

    /// The meaning of a message on a request stream of the pet.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm for each answer of each kind of request, as one match"
    )]
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
            (Purpose::Announce(room), RequestEvent::PublishNamespaceEstablished) => {
                let keeping = self.ask_again.keeping();
                if let Some(here) = self.here.get_mut(room) {
                    here.announced = true;
                    decide(Happened::Subscribed, keeping, &mut here.announce_tries);
                }
                println!(
                    "ANNOUNCED room={room:?} namespace={}",
                    tracks::mine_text(room)
                );
            }
            (Purpose::Garden(room), RequestEvent::SubscribeEstablished { .. }) => {
                let room = room.clone();
                self.garden_established(token, &room)?;
            }
            (Purpose::Publish(room, mine), RequestEvent::PublishEstablished { forward, .. }) => {
                let key = (room.clone(), *mine);
                self.published(token, &key, forward);
            }
            (Purpose::Publish(..), RequestEvent::SubscriptionUpdated { response, .. }) => {
                self.tell(token, Command::Say(response));
            }
            // §10.9.1: the update is refused, and the PUBLISH ends with
            // UPDATE_FAILED once its streams are closed (§10.11).
            (Purpose::Publish(room, mine), RequestEvent::UpdateRefused { response }) => {
                let key = (room.clone(), *mine);
                self.tell(token, Command::Say(response));
                if self.publishing.is_publish(&key, token) {
                    self.end_publish(&key, publish_done_status::UPDATE_FAILED, false);
                }
            }
            (Purpose::Announce(room) | Purpose::Garden(room), RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                let end = Happened::Refused {
                    error_code: error.error_code,
                    named_ms: error.retry_wait_ms(),
                };
                let asked = if matches!(purpose, Purpose::Announce(_)) {
                    Asked::Announce
                } else {
                    Asked::Garden
                };
                self.kept_over(room, asked, end, &why);
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
            (Purpose::Garden(room), RequestEvent::SubscriptionDone(done)) => {
                println!(
                    "SUBSCRIPTION_DONE room={room:?} status={:#x}",
                    done.status_code
                );
                self.let_go(token, Command::Finish);
                self.kept_over(room, Asked::Garden, Happened::Ended, "PUBLISH_DONE");
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
            // LateMessage, the `FETCH_OK`, and what is no answer to a
            // request of its kind.
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

    /// A FETCH of the relay for the `state` of a room: the latest state the
    /// pet holds ([`xmoq_pet::rooms::Room::fetch`]). One for `acted` is
    /// refused with `DOES_NOT_EXIST`: what the pet did is taken live, and
    /// is not done again for one who comes later.
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
        let Some(GardenTrack::State { room }) = tracks::track_of(&full_track_name) else {
            return self.refuse(asked, request_error_code::DOES_NOT_EXIST);
        };
        if self.requests.full() {
            return self.refuse(asked, request_error_code::EXCESSIVE_LOAD);
        }
        let group_order = answers::group_order(&asked.fetch);
        let Some(entry) = self.rooms.get(&room) else {
            return self.refuse(asked, request_error_code::DOES_NOT_EXIST);
        };
        let (planned, states) = match entry.fetch(start, end, group_order) {
            Ok(planned) => planned,
            Err(code) => {
                println!("FETCH_REFUSED room={room:?} error_code={code:#x}");
                return self.refuse(asked, code);
            }
        };
        let count = states.len();
        let pieces = pieces_of(states, planned.left_out);
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
            "FETCH_ANSWERED room={room:?} start={},{} end={},{} states={count}",
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
