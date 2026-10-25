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

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use xmoq_chat::args::Rooms as Taken;
use xmoq_chat::gather::Left;
use xmoq_chat::said_again::{Over, SaidAgain, Settings as AskAgain};
use xmoq_moqt::params::SubscriptionFilter;

use anyhow::{Result, anyhow, bail};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use xmoq_chat::gather::{Admitted, Gatherer};
use xmoq_chat_tracks::names::{self, Person, lines_track, person_of, room_of_lines, said_track};
use xmoq_chat_tracks::payloads::Line;
use xmoq_moqt::data::ObjectStatus;
use xmoq_moqt::early::{Arrived, EarlyObjects, held_bytes};
use xmoq_moqt::immutable::TrackRefusal;
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, Fetch, FetchRange, ReasonPhrase, RequestOk, publish_done_status,
    request_error_code, setup_option,
};
use xmoq_moqt::name::{Location, TrackNamespace};
use xmoq_moqt::params::{ANSWER_WAIT_MS, GroupOrder, Parameter, answer_wait_ms};
use xmoq_moqt::session::{
    ControlEvent, PeerRequest, PeerRequestEvent, PeerRequestId, PeerRequestOpened, RequestEvent,
    RequestId, Role, Session, SubscribedAlias, Transport,
};
use xmoq_moqt::token::Token;
use xmoq_moqt::{RequestFault, stream_reset_code};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, PeerRequestClose, PeerRequestStreamEvent, RequestClose, RequestReadError,
    SendStream, StreamConfig, StreamEnd, StreamStops, Trace, UniEvent, read_request_streams,
    read_uni_streams, send_message,
};
use xmoq_room_token::{Role as TokenRole, RoomKey};

use xmoq_track_serving::backoff::Backoff;
use xmoq_track_serving::fetch::{Held, payload_bytes, pieces_of, plan};
use xmoq_track_serving::streams::{
    Answer, Answered, Answering, Command, GroupToSend, Heard, RequestTask, Sending, Waits,
    answer_fetch, open_request, send_group,
};
use xmoq_track_serving::{shown, sleep_until};

use crate::storage::Store;

type Send = <WtConnection as Connection>::Send;

/// How long the relay's SETUP is waited for, as the Rust client and
/// `@xmoq/moqt` wait. Then the session is closed with
/// `CONTROL_MESSAGE_TIMEOUT` (§3.5).
const SETUP_WAIT: Duration = Duration::from_secs(10);

/// How long a token the gatherer makes runs, in seconds: as long as one of
/// the room (`TOKEN_SECONDS` of `spikes/durable-objects-room/worker.js`).
const TOKEN_SECONDS: u64 = 600;

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

/// Subgroup streams of lines being written at once. Past it a line is not
/// sent live; as above. WHY a limit: a relay that takes nothing would have
/// a task for every line. Provisional.
const LINES_BEING_SENT: usize = 256;

/// FETCH requests answered at once, as many as the Rust client answers
/// (`MAX_ANSWERED_AT_ONCE`); one more is refused with `EXCESSIVE_LOAD`.
/// WHY twice the 16 the relay asks one publisher for: as there. A FETCH
/// that was answered and waits for the relay to end its request stream is
/// not counted; what bounds those is the session, which keeps no more than
/// 256 requests of the relay (`MAX_PEER_REQUESTS`). Provisional.
const FETCHES_AT_ONCE: usize = 32;

/// The most bytes of payload the answers to FETCH requests that are being
/// written hold together ([`Held`]).
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

/// How long a room whose PUBLISH failed waits before it is published
/// again, on the next line that comes for it. Provisional.
const PUBLISH_AGAIN_AFTER: Duration = Duration::from_secs(5);

/// The Track Alias of the first room the gatherer publishes; each next one
/// has the next (§11.1: one alias a track of the session).
const FIRST_TRACK_ALIAS: u64 = 1;

/// How a session ended without an error.
#[derive(Debug)]
pub enum Ended {
    /// The gatherer was told to stop.
    Stopped,
    /// The relay sent GOAWAY (§10.4).
    GoAway,
}

/// The relay sent no SETUP in time.
#[derive(Debug)]
pub struct NoSetupInTime;

impl std::fmt::Display for NoSetupInTime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the relay sent no SETUP within {SETUP_WAIT:?}")
    }
}

impl std::error::Error for NoSetupInTime {}

/// A participant that is followed in this session.
struct Followed {
    /// The token of its SUBSCRIBE, while it has one.
    said: Option<u64>,
    /// When it is asked again after its subscription is over.
    again: SaidAgain,
}

/// What a request of the gatherer is for.
#[derive(Debug, Clone)]
enum Purpose {
    /// A `SUBSCRIBE_NAMESPACE`: for `xmoq/chat`, or for `xmoq/chat/<room>`
    /// of each `--room`, by its place in [`Gathering::watches`].
    Watch(usize),
    /// The SUBSCRIBE for `said` of a participant.
    Said(Person),
    /// The PUBLISH of `lines` of a room.
    Lines(String),
}

/// A request of the gatherer, by the token its task reports with.
struct Own {
    purpose: Purpose,
    request: RequestId,
    task: RequestTask,
}

/// One `SUBSCRIBE_NAMESPACE` of the gatherer, and when it is asked for again
/// after it ended.
struct Watch {
    prefix: TrackNamespace,
    again: Option<Instant>,
    sent: Instant,
    backoff: Backoff,
}

/// A PUBLISH of a room that is to end: its `PUBLISH_DONE` waits for every
/// subgroup stream of the room to be closed (§10.11: "A sender MUST NOT send
/// `PUBLISH_DONE` until it has closed all streams it will ever open"), and
/// then counts them (Stream Count).
struct Ending {
    room: String,
    status: u64,
    opened: Arc<AtomicU64>,
}

/// A room the gatherer numbers.
struct Room {
    /// The token of its PUBLISH, while there is one.
    publish: Option<u64>,
    track_alias: u64,
    /// Whether the PUBLISH has its `PUBLISH_OK` with Forward State 1: lines
    /// are sent on it.
    live: bool,
    /// The subgroup streams of its PUBLISH that were opened.
    opened: Arc<AtomicU64>,
    /// Lines that wait for the `PUBLISH_OK`.
    waiting: VecDeque<Line>,
    /// Not published again before this, after a PUBLISH that failed.
    not_before: Option<Instant>,
    /// The lines of the room whose streams are being sent ([`Sending`]).
    sending: watch::Sender<usize>,
}

impl Room {
    fn new() -> Self {
        Self {
            publish: None,
            track_alias: 0,
            live: false,
            opened: Arc::new(AtomicU64::new(0)),
            waiting: VecDeque::new(),
            not_before: None,
            sending: watch::channel(0).0,
        }
    }
}

/// A FETCH of the relay that the gatherer answers.
struct Fetching {
    request: PeerRequestId,
    answering: Answering,
    /// Its answer was written, or ended otherwise.
    over: bool,
    /// The relay finished its direction of the request stream.
    peer_finished: bool,
    /// The bytes of payload the answer holds until it is written ([`Held`]).
    bytes: usize,
}

/// Everything of one session.
struct Gathering<'a> {
    connection: &'a Arc<WtConnection>,
    store: &'a Store,
    gatherer: &'a mut Gatherer,
    session: Session,
    stops: StreamStops,
    own: HashMap<u64, Own>,
    next_token: u64,
    heard: mpsc::Sender<(u64, Heard)>,
    rooms: HashMap<String, Room>,
    next_track_alias: u64,
    /// Each participant that is followed.
    people: HashMap<Person, Followed>,
    /// The participants whose `said` is to be asked for again, and when.
    /// WHY apart from [`Self::people`]: the loop looks for the earliest
    /// with every event, and few wait at once out of up to 4,096 followed.
    asking_again: HashMap<Person, Instant>,
    /// `--ask-again-times`, `--ask-again-ms`.
    ask_again: AskAgain,
    /// Objects of `said` read before the `SUBSCRIBE_OK` that gives their
    /// Track Alias, by that alias, with their payload (`xmoq_moqt::early`).
    early: EarlyObjects<(u64, Vec<u8>)>,
    /// The requests of the relay the session keeps after the gatherer
    /// refused them, by their stream, until the stream is gone.
    refused: HashMap<u64, PeerRequestId>,
    fetches: HashMap<u64, Fetching>,
    fetch_done: mpsc::Sender<(u64, Answered)>,
    held: Held,
    /// The PUBLISH requests that wait for their streams to be closed, by
    /// token, and where the loop is told that they are.
    ending: HashMap<u64, Ending>,
    drained: mpsc::Sender<u64>,
    watches: Vec<Watch>,
    /// The rooms the gatherer takes (`--room`).
    taken: &'a Taken,
    /// `--auth-key-hex`.
    key: Option<&'a RoomKey>,
    sends: JoinSet<()>,
    /// Answers to requests of the relay that are being written.
    replies: JoinSet<()>,
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
/// code (`termination_code` of `main.rs`).
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
            .map(|room| names::room_namespace(room))
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
    store: &Store,
    (gatherer, rooms_taken, key): (&mut Gatherer, &Taken, Option<&RoomKey>),
    ask_again: AskAgain,
    stop: &mut watch::Receiver<bool>,
) -> Result<Ended> {
    let stops = StreamStops::default();
    let (uni_queue, mut uni) = mpsc::channel(UNI_QUEUE);
    let stream_config = StreamConfig {
        subgroup_buffer_limit: xmoq_chat::args::PAYLOAD_MOST,
        ..StreamConfig::default()
    };
    let _uni_readers = read_uni_streams(
        Arc::clone(connection),
        stream_config.clone(),
        stops.clone(),
        uni_queue,
        |event| event,
    );
    let (peer_queue, mut peer) = mpsc::channel(PEER_REQUEST_QUEUE);
    let _request_readers = read_request_streams(
        Arc::clone(connection),
        stream_config,
        stops.clone(),
        peer_queue,
        |event| event,
    );
    let (heard, mut heard_by_loop) = mpsc::channel(HEARD_QUEUE);
    // Each FETCH says two things to the loop, and waits when it is full.
    let (fetch_done, mut fetches_done) = mpsc::channel(FETCHES_AT_ONCE);
    let (drained, mut drained_rooms) = mpsc::channel(PEER_REQUEST_QUEUE);
    let mut gathering = Gathering {
        connection,
        store,
        gatherer,
        session: Session::new(Role::Client, Transport::WebTransport),
        stops,
        own: HashMap::new(),
        next_token: 0,
        heard,
        rooms: HashMap::new(),
        next_track_alias: FIRST_TRACK_ALIAS,
        people: HashMap::new(),
        asking_again: HashMap::new(),
        ask_again,
        early: EarlyObjects::default(),
        refused: HashMap::new(),
        fetches: HashMap::new(),
        fetch_done,
        held: Held::new(MAX_BYTES_HELD),
        ending: HashMap::new(),
        drained,
        watches: prefixes(rooms_taken)?
            .into_iter()
            .map(|prefix| Watch {
                prefix,
                again: None,
                sent: Instant::now(),
                backoff: Backoff::default(),
            })
            .collect(),
        taken: rooms_taken,
        key,
        sends: JoinSet::new(),
        replies: JoinSet::new(),
        began: Instant::now(),
    };

    // §3.3: each peer opens one control stream that begins with SETUP, and
    // keeps it open for the session.
    let mut control = connection.open_uni().await?;
    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    // In ascending order of their types: the token (0x03), then the
    // implementation (0x07). The relay looks at no namespace or role of the
    // token of a SETUP.
    let mut options: Vec<KeyValuePair> = gathering
        .token(TokenRole::Sub)
        .map(|token| KeyValuePair::bytes(setup_option::AUTHORIZATION_TOKEN, token))
        .transpose()?
        .into_iter()
        .collect();
    options.push(KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        implementation.as_bytes().to_vec(),
    )?);
    let setup = gathering.session.setup(options)?;
    send_message(&mut control, &setup, &Trace::default(), "control").await?;
    match tokio::time::timeout(SETUP_WAIT, uni.recv()).await {
        Ok(Some(UniEvent::Control(message))) => {
            gathering.session.on_control_message(message)?;
        }
        // WHY the error itself, not a line about it: the session is closed
        // with its code, as a datagram of an unknown type that comes before
        // the SETUP has it (§11).
        Ok(Some(UniEvent::Failed(error))) => return Err(error.into()),
        Ok(other) => bail!("no SETUP from the relay: {other:?}"),
        Err(_) => return Err(NoSetupInTime.into()),
    }
    println!("SETUP_OK");
    // Who is in the rooms is what this session is told; the numbering of
    // the rooms goes on.
    gathering.gatherer.leave_everybody();
    for index in 0..gathering.watches.len() {
        gathering.watch(index)?;
    }

    let ended = loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break Ended::Stopped;
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
            Some((stream, answered)) = fetches_done.recv() => gathering.fetch_over(stream, answered),
            Some(token) = drained_rooms.recv() => gathering.publish_done(token)?,
            () = sleep_until(gathering.next_watch()) => gathering.watch_again()?,
            () = sleep_until(gathering.next_said()) => gathering.said_again()?,
            Some(_) = gathering.sends.join_next() => {}
            Some(_) = gathering.replies.join_next() => {}
        }
    };
    // What is open goes with the session, which the caller closes.
    drop(control);
    Ok(ended)
}

impl Gathering<'_> {
    /// A token of the gatherer's own for `role` in every namespace of the
    /// chat (`xmoq/chat/*`) that runs for [`TOKEN_SECONDS`] from now,
    /// carried as an AUTHORIZATION TOKEN (§10.2.2: Alias Type `USE_VALUE`,
    /// Token Type 0); `None` without `--auth-key-hex`.
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
        let text = key.make(&namespace, role, now.saturating_add(TOKEN_SECONDS));
        Some(
            Token::UseValue {
                token_type: 0,
                value: text.into_bytes(),
            }
            .encode(),
        )
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
        watch.again = None;
        watch.sent = Instant::now();
        let prefix = watch.prefix.clone();
        let (request, message) = self.session.subscribe_namespace(
            prefix,
            self.token_parameter(TokenRole::Sub).into_iter().collect(),
        )?;
        self.open(Purpose::Watch(index), request, message, &[]);
        Ok(())
    }

    /// When the next watch that ended is to be asked for again.
    fn next_watch(&self) -> Option<Instant> {
        self.watches.iter().filter_map(|watch| watch.again).min()
    }

    /// Asks again for every watch whose time has come.
    fn watch_again(&mut self) -> Result<()> {
        let now = Instant::now();
        let due: Vec<usize> = (0..self.watches.len())
            .filter(|index| {
                self.watches
                    .get(*index)
                    .and_then(|watch| watch.again)
                    .is_some_and(|again| again <= now)
            })
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
        let token = self.next_token;
        self.next_token += 1;
        let waits = Waits {
            open: OPEN_WAIT,
            answer: Duration::from_millis(answer_wait_ms(ANSWER_WAIT_MS, parameters)),
        };
        let task = open_request(self.connection, token, message, waits, self.heard.clone());
        self.own.insert(
            token,
            Own {
                purpose,
                request,
                task,
            },
        );
        token
    }

    /// Tells the task of a request stream something.
    ///
    /// WHY `try_send` and not a wait: the loop must not wait on one stream.
    /// WHY no command is ever dropped: among them are the answers to the
    /// relay's `REQUEST_UPDATE`s, and §10.9 has "The receiver of a
    /// `REQUEST_UPDATE` MUST respond with exactly one `REQUEST_OK` or
    /// `REQUEST_ERROR`". Where the queue is full ([`xmoq_track_serving::streams::COMMAND_QUEUE`]),
    /// the relay takes nothing on the stream and sends more than it reads:
    /// the request is cancelled with `EXCESSIVE_LOAD` (§3.3.3), as the own
    /// relay ends a FETCH with too many updates waiting.
    fn tell(&mut self, token: u64, command: Command) {
        let full = self
            .own
            .get(&token)
            .is_some_and(|own| own.task.commands.try_send(command).is_err());
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
        let mut own = self.own.remove(&token)?;
        match command {
            Command::Cancel(code) => {
                self.session.cancel(own.request);
                if let Some(abort) = own.task.abort.take() {
                    let _ = abort.send(code);
                }
            }
            // Queued before the sender is dropped, it is read before the
            // task sees the end of the queue; where it does not fit, the
            // dropped sender cancels the stream.
            command => {
                let _ = own.task.commands.try_send(command);
            }
        }
        Some(own.purpose)
    }

    /// Publishes the `lines` of a room, unless they are published or wait
    /// to be published again.
    fn publish(&mut self, room: &str) -> Result<()> {
        let entry = self.rooms.entry(room.to_string()).or_insert_with(Room::new);
        if entry.publish.is_some() || entry.not_before.is_some_and(|after| Instant::now() < after) {
            return Ok(());
        }
        let track_alias = self.next_track_alias;
        self.next_track_alias += 1;
        // §10.2.11: the largest line the gatherer has, which the relay gives
        // its subscribers in SUBSCRIBE_OK, and where a Joining FETCH ends.
        let parameters: Vec<Parameter> = self
            .gatherer
            .rooms
            .largest(room)
            .map(|group| Parameter::LargestObject(Location { group, object: 0 }))
            .into_iter()
            .chain(self.token_parameter(TokenRole::Pub))
            .collect();
        let published =
            self.session
                .publish(lines_track(room)?, track_alias, parameters.clone(), vec![]);
        let Ok((request, message)) = published else {
            // The PUBLISH before it still waits for its streams to be
            // closed; it is published again once that is over.
            println!("ROOM_NOT_PUBLISHED room={room:?} why=\"the PUBLISH before is ending\"");
            return Ok(());
        };
        let token = self.open(
            Purpose::Lines(room.to_string()),
            request,
            message,
            &parameters,
        );
        if let Some(entry) = self.rooms.get_mut(room) {
            entry.publish = Some(token);
            entry.track_alias = track_alias;
            entry.live = false;
            entry.opened = Arc::new(AtomicU64::new(0));
            entry.not_before = None;
        }
        Ok(())
    }

    /// The PUBLISH of a room is over; the next line publishes it again.
    /// Does nothing where `token` is no longer the room's PUBLISH.
    fn unpublish(&mut self, room: &str, token: u64, failed: bool) {
        if let Some(entry) = self.rooms.get_mut(room)
            && entry.publish == Some(token)
        {
            entry.publish = None;
            entry.live = false;
            entry.waiting.clear();
            entry.not_before = failed.then(|| Instant::now() + PUBLISH_AGAIN_AFTER);
            println!("ROOM_UNPUBLISHED room={room:?}");
        }
    }

    /// Ends the PUBLISH of a room with `status`: no line is sent on it from
    /// now on, and its `PUBLISH_DONE` goes once the streams being sent are
    /// closed ([`Self::publish_done`]). `forget`: the room is numbered no
    /// more, and its state goes.
    fn end_publish(&mut self, room: &str, status: u64, forget: bool) {
        let Some(entry) = self.rooms.get_mut(room) else {
            return;
        };
        let token = entry.publish.take();
        entry.live = false;
        entry.waiting.clear();
        let sending = entry.sending.subscribe();
        let opened = Arc::clone(&entry.opened);
        if forget {
            self.rooms.remove(room);
        }
        let Some(token) = token else {
            return;
        };
        self.ending.insert(
            token,
            Ending {
                room: room.to_string(),
                status,
                opened,
            },
        );
        let drained = self.drained.clone();
        tokio::spawn(async move {
            let mut sending = sending;
            // An error: the room's state went with its last sender, and no
            // stream of it is being sent.
            let _ = sending.wait_for(|count| *count == 0).await;
            let _ = drained.send(token).await;
        });
    }

    /// Every stream of an ending PUBLISH is closed: its `PUBLISH_DONE`, with
    /// the number of streams it opened (§10.11), and a FIN. Where the room
    /// is numbered again meanwhile, it is published anew.
    fn publish_done(&mut self, token: u64) -> Result<()> {
        let Some(ending) = self.ending.remove(&token) else {
            return Ok(());
        };
        if let Some(own) = self.own.get(&token) {
            let done = self.session.publish_done(
                own.request,
                ending.status,
                ending.opened.load(Ordering::Relaxed),
                ReasonPhrase::default(),
            )?;
            self.tell(token, Command::Say(done));
            self.let_go(token, Command::Finish);
            println!(
                "ROOM_DONE room={:?} status={:#x} streams={}",
                ending.room,
                ending.status,
                ending.opened.load(Ordering::Relaxed)
            );
        }
        if self.gatherer.rooms.knows(&ending.room) {
            if let Some(entry) = self.rooms.get_mut(&ending.room) {
                entry.not_before = None;
            }
            self.publish(&ending.room)?;
        }
        Ok(())
    }

    /// Subscribes to what a participant says.
    ///
    /// WHY the Largest Object filter (§5.1.2: "The filter Start Location is
    /// {Largest Object.Group, Largest Object.Object + 1}"): without a filter
    /// "Subscriptions without a filter pass all Objects published or
    /// received via upstream subscriptions" (§5.1.2, §10.2.9: "If omitted
    /// from SUBSCRIBE ... the subscription is unfiltered"), and a relay that
    /// still has a participant's lines would pass them again to a gatherer
    /// that connects anew, which numbers what comes and would keep each
    /// twice. WHY NOT remember the Group IDs taken of each participant:
    /// a participant that comes again numbers its Groups anew (the own Rust
    /// client from 0), and its new lines would be taken for old ones. What
    /// a participant said while the gatherer was away is lost either way.
    fn subscribe(&mut self, person: &Person) -> Result<()> {
        let mut parameters = vec![
            Parameter::SubscriptionFilter(SubscriptionFilter::LargestObject),
            Parameter::Forward(true),
            Parameter::SubscriberPriority(128),
            Parameter::GroupOrder(GroupOrder::Ascending),
        ];
        parameters.extend(self.token_parameter(TokenRole::Sub));
        self.asking_again.remove(person);
        let Ok((request, message)) = self
            .session
            .subscribe(said_track(person)?, parameters.clone())
        else {
            // A subscription to the track is there already.
            println!(
                "NOT_SUBSCRIBED room={:?} name={:?} why=\"a subscription is there already\"",
                person.room, person.name
            );
            return Ok(());
        };
        let token = self.open(Purpose::Said(person.clone()), request, message, &parameters);
        if let Some(followed) = self.people.get_mut(person) {
            followed.said = Some(token);
        }
        Ok(())
    }

    /// The `said` of a participant that stays announced is over: it is
    /// asked for again after a pause, or not until its NAMESPACE comes
    /// again ([`SaidAgain`]).
    fn said_over(&mut self, person: &Person, over: Over, why: &str) {
        println!(
            "NOT_SUBSCRIBED room={:?} name={:?} why={why:?}",
            person.room, person.name
        );
        // Not followed: its NAMESPACE_DONE came.
        let Some(followed) = self.people.get_mut(person) else {
            return;
        };
        followed.said = None;
        match followed.again.over(over) {
            Some(pause) => {
                println!(
                    "ASKING_AGAIN room={:?} name={:?} in_ms={pause}",
                    person.room, person.name
                );
                self.asking_again.insert(
                    person.clone(),
                    Instant::now() + Duration::from_millis(pause),
                );
            }
            None => println!(
                "NOT_ASKING_AGAIN room={:?} name={:?}",
                person.room, person.name
            ),
        }
    }

    /// The subscription to a participant's `said` is established: the
    /// Objects that came before it are taken.
    fn said_established(&mut self, person: &Person) -> Result<()> {
        println!("SUBSCRIBED room={:?} name={:?}", person.room, person.name);
        if let Some(followed) = self.people.get_mut(person) {
            followed.again.subscribed();
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
            let person = match self.session.subscribed_alias(track_alias) {
                SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => {
                    person_of(track.namespace())
                }
                SubscribedAlias::Unknown | SubscribedAlias::Retired => None,
            };
            if let Some(person) = person {
                self.on_said(&person, &payload)?;
            }
        }
        Ok(())
    }

    /// When the next participant is to be asked for again.
    fn next_said(&self) -> Option<Instant> {
        self.asking_again.values().min().copied()
    }

    /// Asks again for every participant whose time has come.
    fn said_again(&mut self) -> Result<()> {
        let now = Instant::now();
        let due: Vec<Person> = self
            .asking_again
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(person, _)| person.clone())
            .collect();
        for person in due {
            self.asking_again.remove(&person);
            if self
                .people
                .get(&person)
                .is_some_and(|followed| followed.said.is_none())
            {
                self.subscribe(&person)?;
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
                    person.room, person.name
                );
                return Ok(());
            }
        }
        self.publish(&person.room)?;
        let admitted = self.gatherer.admit(&person);
        if matches!(admitted, Admitted::Now | Admitted::Already) {
            // A NAMESPACE that comes again counts the times anew.
            let again = SaidAgain::new(self.ask_again);
            self.people
                .entry(person.clone())
                .and_modify(|followed| followed.again = again.clone())
                .or_insert(Followed { said: None, again });
        }
        match admitted {
            Admitted::Now => {
                println!("PARTICIPANT room={:?} name={:?}", person.room, person.name);
                self.subscribe(&person)
            }
            // Followed, and with no subscription in this session.
            Admitted::Already
                if self
                    .people
                    .get(&person)
                    .is_some_and(|followed| followed.said.is_none()) =>
            {
                self.subscribe(&person)
            }
            Admitted::Already => Ok(()),
            Admitted::TooManyPeople | Admitted::NoRoom => {
                println!(
                    "PARTICIPANT_IGNORED room={:?} name={:?} why=too-many-people",
                    person.room, person.name
                );
                Ok(())
            }
        }
    }

    /// A `NAMESPACE_DONE` under `xmoq/chat`: the participant is gone.
    fn on_namespace_done(&mut self, namespace: &TrackNamespace) {
        let Some(person) = person_of(namespace) else {
            return;
        };
        self.asking_again.remove(&person);
        if let Some(Followed {
            said: Some(token), ..
        }) = self.people.remove(&person)
        {
            self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
        }
        match self.gatherer.leave(&person) {
            Left::NotFollowed => {}
            Left::RoomStays => {
                println!(
                    "PARTICIPANT_GONE room={:?} name={:?}",
                    person.room, person.name
                );
            }
            Left::RoomEmpty => {
                println!(
                    "PARTICIPANT_GONE room={:?} name={:?}",
                    person.room, person.name
                );
                self.end_publish(&person.room, publish_done_status::TRACK_ENDED, true);
            }
        }
    }

    /// An Object of `said`: a line.
    fn on_said(&mut self, person: &Person, payload: &[u8]) -> Result<()> {
        let now_ms = u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX);
        let line = match self.gatherer.take(person, payload, now_ms, wall_ms()) {
            Ok(line) => line,
            Err(dropped) => {
                println!(
                    "LINE_DROPPED room={:?} name={:?} why={} dropped={}",
                    person.room,
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
                person.room, person.name
            );
            return Ok(());
        }
        println!(
            "LINE room={:?} number={} name={:?} text_bytes={}",
            person.room,
            line.number,
            line.name,
            line.text.len()
        );
        self.send_live(&person.room, line)
    }

    /// Sends a line that is kept on the `lines` of its room, or has it wait
    /// for the room's `PUBLISH_OK`.
    fn send_live(&mut self, room: &str, line: Line) -> Result<()> {
        let is_live = self.rooms.get(room).is_some_and(|entry| entry.live);
        if !is_live {
            self.publish(room)?;
            if let Some(entry) = self.rooms.get_mut(room)
                && entry.publish.is_some()
            {
                if entry.waiting.len() >= LINES_WAITING_FOR_A_ROOM {
                    entry.waiting.pop_front();
                }
                entry.waiting.push_back(line);
            }
            return Ok(());
        }
        let Some(entry) = self.rooms.get(room) else {
            return Ok(());
        };
        if self.sends.len() >= LINES_BEING_SENT {
            println!(
                "LINE_NOT_SENT room={room:?} number={} why=too-many-at-once",
                line.number
            );
            return Ok(());
        }
        let sending = GroupToSend {
            track_alias: entry.track_alias,
            group_id: line.number,
            payload: line.payload(),
            opened: Arc::clone(&entry.opened),
        };
        let counted = Sending::begin(&entry.sending);
        let connection = Arc::clone(self.connection);
        let room = room.to_string();
        self.sends.spawn(async move {
            let _counted = counted;
            let number = sending.group_id;
            if let Err(why) = send_group(connection, sending).await {
                println!("LINE_NOT_SENT room={room:?} number={number} why={why:?}");
            }
        });
        Ok(())
    }

    /// An event of the relay's unidirectional streams.
    fn on_uni(&mut self, event: UniEvent) -> Result<Option<Ended>> {
        // What a SUBSCRIBE that ended meanwhile left held is let go of before
        // what came after it.
        self.settle_early()?;
        match event {
            UniEvent::Control(message) => match self.session.on_control_message(message)? {
                ControlEvent::GoAway(_) => {
                    println!("GOAWAY");
                    return Ok(Some(Ended::GoAway));
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
                let person = match self.session.subscribed_alias(alias) {
                    SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => {
                        person_of(track.namespace())
                    }
                    SubscribedAlias::Unknown | SubscribedAlias::Retired => None,
                };
                if let Some(person) = person {
                    self.on_said(&person, &payload)?;
                }
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
            UniEvent::ReaderPanicked => bail!("a reader of a stream panicked"),
        }
        Ok(None)
    }

    /// What the task of one of the gatherer's request streams heard.
    fn on_heard(&mut self, token: u64, heard: Heard) -> Result<Option<Ended>> {
        let Some(own) = self.own.get(&token) else {
            return Ok(None);
        };
        let (purpose, request) = (own.purpose.clone(), own.request);
        match heard {
            Heard::Opened => Ok(None),
            Heard::NotOpened(why) => {
                self.session.cancel(request);
                self.own.remove(&token);
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
            Heard::Closed(RequestClose::Fin) if matches!(purpose, Purpose::Lines(_)) => {
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

    /// A request of the gatherer is over, and was let go of.
    ///
    /// WHY the `SUBSCRIBE_NAMESPACE` is asked for again alone, after a wait
    /// that grows ([`Backoff`]), and the session goes on: what the session
    /// does for the participants and the rooms is not over with it. A relay
    /// that ends it every time (one that tells more namespaces than the
    /// session keeps, `TooManyNamespaces`) is asked no more often than every
    /// 30 seconds. What it misses: a `NAMESPACE_DONE` that comes while no
    /// request is open; that participant stays followed, and is asked again
    /// for its `said` as many times as [`SaidAgain`] lets it.
    fn request_over(&mut self, token: u64, purpose: &Purpose, why: &str) {
        match purpose {
            Purpose::Watch(index) => {
                if let Some(watch) = self.watches.get_mut(*index) {
                    let wait = watch.backoff.after(watch.sent.elapsed());
                    println!(
                        "WATCH_ENDED prefix={} why={why:?} again_in_ms={}",
                        shown(&watch.prefix),
                        wait.as_millis()
                    );
                    watch.again = Some(Instant::now() + wait);
                }
            }
            Purpose::Said(person) => self.said_over(person, Over::Ended, why),
            Purpose::Lines(room) => {
                let room = room.clone();
                println!("ROOM_NOT_PUBLISHED room={room:?} why={why:?}");
                self.unpublish(&room, token, true);
                self.ending.remove(&token);
            }
        }
    }

    /// The meaning of a message on a request stream of the gatherer.
    fn on_answer(
        &mut self,
        token: u64,
        purpose: &Purpose,
        event: RequestEvent,
    ) -> Result<Option<Ended>> {
        match (purpose, event) {
            (_, RequestEvent::GoAway(_)) => {
                println!("GOAWAY on a request stream");
                return Ok(Some(Ended::GoAway));
            }
            (Purpose::Watch(index), RequestEvent::NamespacesSubscribed) => {
                if let Some(watch) = self.watches.get(*index) {
                    println!("WATCHING prefix={}", shown(&watch.prefix));
                }
            }
            (Purpose::Watch(_), RequestEvent::Namespace(namespace)) => {
                self.on_namespace(&namespace)?;
            }
            (Purpose::Watch(_), RequestEvent::NamespaceDone(namespace)) => {
                self.on_namespace_done(&namespace);
            }
            (Purpose::Said(person), RequestEvent::SubscribeEstablished { .. }) => {
                self.said_established(person)?;
            }
            (Purpose::Lines(room), RequestEvent::PublishEstablished { forward, .. }) => {
                let room = room.clone();
                let waiting = match self.rooms.get_mut(&room) {
                    // Not for a PUBLISH that is ending, or one before.
                    Some(entry) if entry.publish == Some(token) => {
                        entry.live = forward;
                        std::mem::take(&mut entry.waiting)
                    }
                    _ => VecDeque::new(),
                };
                let largest = self.gatherer.rooms.largest(&room);
                println!(
                    "ROOM_PUBLISHED room={room:?} forward={forward} largest={}",
                    largest.map_or_else(|| "none".to_string(), |largest| largest.to_string())
                );
                for line in waiting {
                    self.send_live(&room, line)?;
                }
            }
            (Purpose::Lines(_), RequestEvent::SubscriptionUpdated { response, .. }) => {
                self.tell(token, Command::Say(response));
            }
            // §10.9.1: the update is refused, and the subscription ends with
            // UPDATE_FAILED.
            // The PUBLISH_DONE goes once the streams of the room being sent
            // are closed (§10.11), and the room is published again then.
            (Purpose::Lines(room), RequestEvent::UpdateRefused { response }) => {
                let room = room.clone();
                self.tell(token, Command::Say(response));
                if self
                    .rooms
                    .get(&room)
                    .is_some_and(|entry| entry.publish == Some(token))
                {
                    self.end_publish(&room, publish_done_status::UPDATE_FAILED, false);
                }
            }
            (Purpose::Said(person), RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                self.said_over(person, Over::Refused(error.retry_wait_ms()), &why);
            }
            (_, RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                self.request_over(token, purpose, &why);
            }
            (Purpose::Said(person), RequestEvent::SubscriptionDone(done)) => {
                println!(
                    "SUBSCRIPTION_DONE room={:?} name={:?} status={:#x}",
                    person.room, person.name, done.status_code
                );
                self.let_go(token, Command::Finish);
                self.said_over(person, Over::Ended, "PUBLISH_DONE");
            }
            (
                _,
                RequestEvent::TrackNotProcessed {
                    refusal: TrackRefusal::Malformed,
                    ..
                },
            ) => {
                // §2.4.2: "it MUST cancel any corresponding subscription".
                self.let_go(token, Command::Cancel(stream_reset_code::MALFORMED_TRACK));
                match purpose {
                    Purpose::Said(person) => {
                        self.said_over(person, Over::Malformed, "the track is malformed");
                    }
                    _ => self.request_over(token, purpose, "the track is malformed"),
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

    /// An event of a request stream the relay opened.
    fn on_peer_request(&mut self, event: PeerRequestStreamEvent<Send>) -> Result<()> {
        match event {
            PeerRequestStreamEvent::Opened {
                stream,
                send,
                first,
            } => match self.session.on_peer_request_opened(first)? {
                PeerRequestOpened::Refused { response, request } => {
                    if let Some(request) = request {
                        self.refused.insert(stream, request);
                    }
                    self.reply(send, response);
                }
                PeerRequestOpened::Request(request, PeerRequest::Fetch(fetch)) => {
                    self.on_fetch(stream, send, request, &fetch)?;
                }
                // The gatherer is asked for nothing else: it announces no
                // namespace, and is the subscriber of no PUBLISH it takes.
                PeerRequestOpened::Request(request, _) => {
                    self.refuse(stream, send, request, request_error_code::NOT_SUPPORTED)?;
                }
            },
            PeerRequestStreamEvent::Message { stream, message } => {
                if let Some(fetching) = self.fetches.get(&stream) {
                    let event = self
                        .session
                        .on_peer_request_message(fetching.request, message)
                        .map_err(fault)?;
                    // A REQUEST_UPDATE of a FETCH changes nothing, and gets
                    // its REQUEST_OK (§10.9).
                    // None is dropped (§10.9: "exactly one"): where the queue
                    // is full the FETCH is cancelled instead (`tell`).
                    if let PeerRequestEvent::UpdateAccepted { response, .. } = event
                        && fetching.answering.says.try_send(response).is_err()
                    {
                        self.end_fetch(stream, stream_reset_code::EXCESSIVE_LOAD);
                    }
                } else if let Some(request) = self.refused.get(&stream) {
                    // What crossed the refusal: the session has to see it, a
                    // REQUEST_UPDATE uses up a Request ID (§10.1).
                    self.session
                        .on_peer_request_message(*request, message)
                        .map_err(fault)?;
                }
            }
            PeerRequestStreamEvent::Closed { stream, how } => {
                if let Some(request) = self.refused.remove(&stream) {
                    self.session.on_peer_request_closed(request);
                }
                self.fetch_closed(stream, how);
            }
            PeerRequestStreamEvent::Failed(error) => return Err(error.into()),
            PeerRequestStreamEvent::ReaderPanicked => {
                bail!("a reader of a request stream panicked")
            }
        }
        Ok(())
    }

    /// Writes the last message on a request stream of the relay, then a FIN
    /// (§3.3.2), in a task of its own.
    fn reply(&mut self, mut send: Send, response: ControlMessage) {
        self.replies.spawn(async move {
            // An error means the relay is gone or stopped the stream.
            let _ = send_message(&mut send, &response, &Trace::default(), "reply").await;
            send.finish();
        });
    }

    /// Refuses a request of the relay with `code`.
    fn refuse(&mut self, stream: u64, send: Send, request: PeerRequestId, code: u64) -> Result<()> {
        let response = self
            .session
            .reject(request, code, 0, ReasonPhrase::default())?;
        self.refused.insert(stream, request);
        self.reply(send, response);
        Ok(())
    }

    /// A FETCH of the relay: for a range of the `lines` of a room, which is
    /// answered from the store ([`xmoq_chat::fetch`]).
    #[expect(
        clippy::too_many_lines,
        reason = "one refusal after the other, each with its code, and the answer"
    )]
    fn on_fetch(
        &mut self,
        stream: u64,
        send: Send,
        request: PeerRequestId,
        fetch: &Fetch,
    ) -> Result<()> {
        // WHY a Joining FETCH is refused: the gatherer has no subscriber of
        // its own to join; the relay turns a viewer's Joining FETCH into a
        // Standalone one before it asks a publisher (`Relay::join` of
        // `crates/relay-core/src/fetch.rs`).
        let FetchRange::Standalone {
            full_track_name,
            start,
            end,
        } = &fetch.range
        else {
            return self.refuse(stream, send, request, request_error_code::NOT_SUPPORTED);
        };
        let Some(room) = room_of_lines(full_track_name).filter(|room| self.taken.has(room)) else {
            return self.refuse(stream, send, request, request_error_code::DOES_NOT_EXIST);
        };
        let answering = self
            .fetches
            .values()
            .filter(|fetching| !fetching.over)
            .count();
        if answering >= FETCHES_AT_ONCE {
            return self.refuse(stream, send, request, request_error_code::EXCESSIVE_LOAD);
        }
        // §10.2.8: "If omitted from FETCH, the receiver uses Ascending".
        let group_order = fetch
            .parameters
            .iter()
            .find_map(|parameter| match parameter {
                Parameter::GroupOrder(order) => Some(*order),
                _ => None,
            })
            .unwrap_or(GroupOrder::Ascending);
        let largest = if self.gatherer.rooms.knows(&room) {
            self.gatherer.rooms.largest(&room)
        } else {
            self.store.next_number(&room)?.checked_sub(1)
        };
        let most = self.gatherer.limits().fetch_lines;
        let planned = match plan(*start, *end, largest, most, group_order) {
            Ok(planned) => planned,
            Err(code) => {
                println!("FETCH_REFUSED room={room:?} error_code={code:#x}");
                return self.refuse(stream, send, request, code);
            }
        };
        let lines = match planned.groups {
            Some(range) => self.store.lines(&room, range, self.held.room()),
            None => Ok(Some(Vec::new())),
        };
        let mut lines = match lines {
            Ok(Some(lines)) => lines,
            // §10.6.2: "EXCESSIVE_LOAD: The responder is overloaded and
            // cannot process the request at this time" (`MAX_BYTES_HELD`).
            Ok(None) => {
                println!("FETCH_REFUSED room={room:?} error_code=0x9 why=too-many-bytes-held");
                return self.refuse(stream, send, request, request_error_code::EXCESSIVE_LOAD);
            }
            Err(error) => {
                println!("FETCH_REFUSED room={room:?} error_code=0x0 store_error={error:#}");
                return self.refuse(stream, send, request, request_error_code::INTERNAL_ERROR);
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
        let bytes = payload_bytes(&pieces);
        // The store gave no more than there is room for.
        if !self.held.take(bytes) {
            return self.refuse(stream, send, request, request_error_code::EXCESSIVE_LOAD);
        }
        // End Of Track 0: the chat goes on.
        let (ok, owed) = self
            .session
            .accept_fetch(request, false, planned.end_location, vec![])?;
        let mut messages = vec![ok];
        messages.extend((0..owed).map(|_| ControlMessage::RequestOk(RequestOk::default())));
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
        );
        let answer = Answer {
            messages,
            request_id: request.value(),
            group_order,
            pieces,
        };
        let answering = answer_fetch(
            self.connection,
            stream,
            send,
            answer,
            self.fetch_done.clone(),
        );
        self.fetches.insert(
            stream,
            Fetching {
                request,
                answering,
                over: false,
                peer_finished: false,
                bytes,
            },
        );
        Ok(())
    }

    /// What the task of a FETCH said.
    fn fetch_over(&mut self, stream: u64, answered: Answered) {
        let Some(fetching) = self.fetches.get_mut(&stream) else {
            return;
        };
        match answered {
            Answered::Written => {
                println!(
                    "FETCH_SERVED request_id={} how=Written",
                    fetching.request.value()
                );
                fetching.over = true;
                self.held.give_back(std::mem::take(&mut fetching.bytes));
                // From here on nothing is answered on the request stream
                // (§10.1: a REQUEST_UPDATE still uses up its Request ID).
                self.session.on_fetch_answer_ended(fetching.request);
                if fetching.peer_finished {
                    let request = fetching.request;
                    self.fetches.remove(&stream);
                    self.session.on_peer_request_closed(request);
                }
            }
            // The gatherer's direction is over and the relay's is not: it is
            // stopped. WHY: a relay that never ends it would keep the request
            // in the session, which holds 256 of the relay's
            // (`MAX_PEER_REQUESTS`), and refuses every one after. The own
            // relay finishes its direction of a FETCH it passed on once the
            // fetch stream reached its FIN (`finish_if_served` of
            // `crates/relay-core/src/fetch.rs`), and stops a subscriber's
            // direction 2 seconds after its own FIN; the gatherer stops the
            // relay's at its own FIN, which is 3 seconds after the end of the
            // fetch stream at the most (row on the request stream of a FETCH
            // that was served to its end, `docs/moqt-implementation-notes.md`).
            Answered::RequestOver { peer_ended: false } => {
                println!(
                    "FETCH_REQUEST_STOPPED request_id={}",
                    fetching.request.value()
                );
                self.end_fetch(stream, stream_reset_code::CANCELLED);
            }
            Answered::RequestOver { peer_ended: true } => {}
            // The task has reset its directions (§5.2: "It MUST reset the
            // bidi request stream and unidirectional data stream").
            Answered::Stopped | Answered::Failed => {
                println!(
                    "FETCH_SERVED request_id={} how={answered:?}",
                    fetching.request.value()
                );
                self.end_fetch(stream, stream_reset_code::CANCELLED);
            }
        }
    }

    /// Ends a FETCH of the relay on both streams with `code`, where they are
    /// not over, and forgets it.
    fn end_fetch(&mut self, stream: u64, code: u32) {
        let Some(mut fetching) = self.fetches.remove(&stream) else {
            return;
        };
        if let Some(cancel) = fetching.answering.cancel.take() {
            let _ = cancel.send(code);
        }
        if !fetching.peer_finished {
            self.stops.stop(stream, code);
        }
        self.held.give_back(fetching.bytes);
        self.session.on_peer_request_closed(fetching.request);
    }

    /// The relay's direction of the request stream of a FETCH is over.
    fn fetch_closed(&mut self, stream: u64, how: PeerRequestClose) {
        let Some(fetching) = self.fetches.get_mut(&stream) else {
            return;
        };
        // A subscriber that finishes its direction has not given the FETCH
        // up (§3.3.2); one that resets it has.
        if !how.is_fin() {
            fetching.peer_finished = true;
            self.end_fetch(stream, stream_reset_code::CANCELLED);
            return;
        }
        fetching.peer_finished = true;
        if let Some(ended) = fetching.answering.peer_ended.take() {
            let _ = ended.send(());
        }
        if fetching.over {
            let request = fetching.request;
            self.fetches.remove(&stream);
            self.session.on_peer_request_closed(request);
        }
    }
}
