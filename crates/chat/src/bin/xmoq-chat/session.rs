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
use xmoq_moqt::message::{
    ControlMessage, FetchRange, ReasonPhrase, publish_done_status, request_error_code,
};
use xmoq_moqt::name::{Location, TrackNamespace};
use xmoq_moqt::params::{ANSWER_WAIT_MS, GroupOrder, Parameter, answer_wait_ms};
use xmoq_moqt::session::{
    ControlEvent, MalformedRequest, RequestEvent, RequestId, Role, Session, SubscribedAlias,
    Transport,
};
use xmoq_moqt::token::Token;
use xmoq_moqt::{RequestFault, stream_reset_code};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, PeerRequestStreamEvent, RequestClose, RequestReadError, StreamConfig, StreamEnd,
    StreamStops, UniEvent, client, read_request_streams, read_uni_streams,
};
use xmoq_room_token::{Role as TokenRole, RoomKey};

use xmoq_again::next_session::new_url;
use xmoq_track_serving::answers::{self, Bounds, MAX_ANSWERED_AT_ONCE, Note};
use xmoq_track_serving::backoff::Backoff;
use xmoq_track_serving::fetch::{pieces_of, plan};
use xmoq_track_serving::publication::{Ending, FIRST_TRACK_ALIAS, Publication};
use xmoq_track_serving::requests::{AskedFetch, FetchAnswer, PeerRequests};
use xmoq_track_serving::streams::{
    COMMAND_QUEUE, Command, GroupToSend, Heard, RequestTask, Sending, Waits, open_request,
    send_group,
};
use xmoq_track_serving::{shown, sleep_until};

use crate::storage::Store;

type Send = <WtConnection as Connection>::Send;

/// How long a token the gatherer makes runs, in seconds: as long as one of
/// the room (`TOKEN_SECONDS` of `packages/room/src/token.ts`).
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

/// The most bytes of payload the answers to FETCH requests that are being
/// written hold together (`Held` of `xmoq-track-serving`).
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
pub enum Ended {
    /// The gatherer was told to stop.
    Stopped,
    /// The relay sent GOAWAY on the control stream (§10.4), with the URL
    /// to go to, or none for the same.
    Moved(Option<String>),
}

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
    /// The `lines` of each room the gatherer numbers.
    rooms: HashMap<String, Publication<Line>>,
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
    /// The requests of the relay: the FETCH requests being answered, and
    /// those refused.
    requests: PeerRequests,
    /// The PUBLISH requests that wait for their streams to be closed, by
    /// token, with their room, and where the loop is told that they are.
    ending: HashMap<u64, (String, Ending)>,
    drained: mpsc::Sender<u64>,
    watches: Vec<Watch>,
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
    kept: &mut client::UntilClosed<WtConnection>,
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
        requests: PeerRequests::new(Bounds {
            at_once: MAX_ANSWERED_AT_ONCE,
            bytes: MAX_BYTES_HELD,
            say_queue: COMMAND_QUEUE,
        }),
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

    let ended = loop {
        // WHY taken before the select: its arms borrow parts of the state
        // mutably, the task notes of the FETCH answers among them.
        let (next_watch, next_said) = (gathering.next_watch(), gathering.next_said());
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
            note = gathering.requests.next_note() => gathering.fetch_noted(note),
            Some(token) = drained_rooms.recv() => gathering.publish_done(token)?,
            () = sleep_until(next_watch) => gathering.watch_again()?,
            () = sleep_until(next_said) => gathering.said_again()?,
            Some(_) = gathering.sends.join_next() => {}
        }
    };
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

    // FIXME: take the steps of a PUBLISH (`publish` to `publish_done`,
    // `send_live`) and of the gatherer's own requests (`open`, `tell`,
    // `let_go`, `watch`, `watch_again`) from `xmoq-track-serving`; the
    // transcriber has a copy of each.
    /// Publishes the `lines` of a room, unless they are published or wait
    /// to be published again.
    fn publish(&mut self, room: &str) -> Result<()> {
        let entry = self
            .rooms
            .entry(room.to_string())
            .or_insert_with(|| Publication::new(LINES_WAITING_FOR_A_ROOM));
        if !entry.to_publish(Instant::now()) {
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
            entry.published(token, track_alias);
        }
        Ok(())
    }

    /// The PUBLISH of a room is over; the next line publishes it again.
    /// Does nothing where `token` is no longer the room's PUBLISH.
    fn unpublish(&mut self, room: &str, token: u64, failed: bool) {
        if let Some(entry) = self.rooms.get_mut(room)
            && entry.unpublished(token, failed, Instant::now())
        {
            println!("ROOM_UNPUBLISHED room={room:?}");
        }
    }

    /// Ends the PUBLISH of a room with `status`: no line is sent on it from
    /// now on, and its `PUBLISH_DONE` goes once the streams being sent are
    /// closed ([`Self::publish_done`]). `forget`: the room is numbered no
    /// more, and its state goes.
    fn end_publish(&mut self, room: &str, status: u64, forget: bool) {
        let ending = self.rooms.get_mut(room).and_then(|entry| entry.end(status));
        if forget {
            self.rooms.remove(room);
        }
        let Some(ending) = ending else {
            return;
        };
        let drained = self.drained.clone();
        let when_drained = ending.when_drained();
        tokio::spawn(async move {
            let _ = drained.send(when_drained.await).await;
        });
        self.ending.insert(ending.token, (room.to_string(), ending));
    }

    /// Every stream of an ending PUBLISH is closed: its `PUBLISH_DONE`, with
    /// the number of streams it opened (§10.11), and a FIN. Where the room
    /// is numbered again meanwhile, it is published anew.
    fn publish_done(&mut self, token: u64) -> Result<()> {
        let Some((room, ending)) = self.ending.remove(&token) else {
            return Ok(());
        };
        if let Some(own) = self.own.get(&token) {
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
                "ROOM_DONE room={room:?} status={:#x} streams={streams}",
                ending.status
            );
        }
        if self.gatherer.rooms.knows(&room) {
            if let Some(entry) = self.rooms.get_mut(&room) {
                entry.may_publish_again();
            }
            self.publish(&room)?;
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
        let is_live = self.rooms.get(room).is_some_and(Publication::live);
        if !is_live {
            self.publish(room)?;
            if let Some(entry) = self.rooms.get_mut(room) {
                entry.wait(line);
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
            track_alias: entry.track_alias(),
            group_id: line.number,
            payload: line.payload(),
            opened: Arc::clone(entry.opened()),
        };
        let counted = Sending::begin(entry.sending());
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
                ControlEvent::GoAway(go_away) => {
                    println!("GOAWAY");
                    return Ok(Some(Ended::Moved(new_url(&go_away.new_session_uri))));
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
            Purpose::Lines(room) => {
                if self
                    .rooms
                    .get(room)
                    .is_some_and(|entry| entry.publish() == Some(token))
                {
                    let room = room.clone();
                    self.end_publish(&room, publish_done_status::GOING_AWAY, false);
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

    /// The track of `request` is malformed, and the session ended the
    /// request: §2.4.2, "it MUST cancel any corresponding subscription or
    /// fetches for that Track". Its stream is reset with `MALFORMED_TRACK`.
    fn malformed(&mut self, request: RequestId) {
        let token = self
            .own
            .iter()
            .find_map(|(token, own)| (own.request == request).then_some(*token));
        let Some(token) = token else {
            return;
        };
        let Some(purpose) = self.let_go(token, Command::Cancel(stream_reset_code::MALFORMED_TRACK))
        else {
            return;
        };
        match &purpose {
            Purpose::Said(person) => {
                self.said_over(person, Over::Malformed, "the track is malformed");
            }
            purpose => self.request_over(token, purpose, "the track is malformed"),
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
                self.request_moved(token, purpose);
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
                let waiting = self
                    .rooms
                    .get_mut(&room)
                    .map(|entry| entry.established(token, forward))
                    .unwrap_or_default();
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
                    .is_some_and(|entry| entry.publish() == Some(token))
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
                let request = self.own.get(&token).map(|own| own.request);
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
        let Some(room) = room_of_lines(&full_track_name).filter(|room| self.taken.has(room)) else {
            return self.refuse(asked, request_error_code::DOES_NOT_EXIST);
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
                println!("FETCH_REFUSED room={room:?} error_code={code:#x}");
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
                println!("FETCH_REFUSED room={room:?} error_code=0x9 why=too-many-bytes-held");
                return self.refuse(asked, request_error_code::EXCESSIVE_LOAD);
            }
            Err(error) => {
                println!("FETCH_REFUSED room={room:?} error_code=0x0 store_error={error:#}");
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
