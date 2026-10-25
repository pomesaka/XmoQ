//! One `MoQ` Transport session of the transcriber with the relay: the
//! broadcasts it is told of, the catalog and the audio track of each, the
//! transcript of each prefix it publishes, and the FETCH requests it
//! answers.
//!
//! **What is listened to** (decided on 2026-10-25): every broadcast
//! announced under a prefix of `--namespace`, found with one
//! `SUBSCRIBE_NAMESPACE` a prefix, and of each the first Opus audio track
//! with LOC packaging its catalog names (MSF §5). WHY NOT every namespace
//! the relay has: whoever announces would have the transcriber's processor.
//! WHY NOT where a room says the broadcast is: that ties the transcriber to
//! the room's WebSocket, and a meeting has a broadcast for each person.
//!
//! **What is published**: the track `speech` of `xmoq/transcript/<prefix>`
//! for each prefix, with PUBLISH, from the start of the session to its end,
//! a piece a Group. WHY PUBLISH and not `PUBLISH_NAMESPACE`: the reasons the
//! gatherer of the chat gives at the top of its `session.rs` (a FETCH
//! reaches the PUBLISH's publisher first, and the relay knows the Largest
//! Object before a viewer asks). WHY for the whole session and not while a
//! broadcast is listened to, as the gatherer publishes a room while
//! somebody is in it: there are 8 prefixes at the most, and a viewer who
//! comes before the broadcast finds the track.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use xmoq_again::AskAgain;
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::{ObjectStatus, SubgroupObject};
use xmoq_moqt::early::{Arrived, EarlyObjects, held_bytes};
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::immutable::{Of, TrackRefusal, searched_integers};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, Fetch, FetchRange, ReasonPhrase, RequestOk, publish_done_status,
    request_error_code, setup_option,
};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::{ANSWER_WAIT_MS, GroupOrder, Parameter, answer_wait_ms};
use xmoq_moqt::session::{
    ControlEvent, FetchStreamOpened, PeerRequest, PeerRequestEvent, PeerRequestId,
    PeerRequestOpened, RequestEvent, RequestId, Role, Session, SubscribedAlias, Transport,
};
use xmoq_moqt::token::Token;
use xmoq_moqt::{RequestFault, stream_reset_code};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, PeerRequestClose, PeerRequestStreamEvent, RequestClose,
    RequestReadError, StreamConfig, StreamEnd, StreamStops, Trace, UniEvent, read_request_streams,
    read_uni_streams, send_message,
};
use xmoq_room_token::{Role as TokenRole, RoomKey};
use xmoq_speech::cut::Cut;
use xmoq_speech::judge::{Judge, ModelJudge, VOICE_DBFS};
use xmoq_speech::vad::Silero;
use xmoq_speech::{AudioPacket, Listener};
use xmoq_track_serving::backoff::Backoff;
use xmoq_track_serving::fetch::{Held, Plan, payload_bytes, pieces_of, plan};
use xmoq_track_serving::streams::{
    Answer, Answered, Answering, Command, GroupToSend, Heard, RequestTask, Sending, Waits,
    answer_fetch, open_request, send_group,
};
use xmoq_track_serving::{shown, sleep_until};
use xmoq_transcriber::args::Args;
use xmoq_transcriber::transcript::Transcript;
use xmoq_transcript_tracks::names::{listened_of, speech_track, transcript_namespace};
use xmoq_transcript_tracks::payloads::Speech;

use crate::reader::{Read, Reader, ToRead};

type Send = <WtConnection as Connection>::Send;

/// A piece as a FETCH sends it: its Group ID and its payload.
type Kept = (u64, Vec<u8>);

/// How long the relay's SETUP is waited for, as the gatherer waits.
const SETUP_WAIT: Duration = Duration::from_secs(10);

/// How long a token the transcriber makes runs, in seconds, as the
/// gatherer's.
const TOKEN_SECONDS: u64 = 600;

/// How long a request may wait for the relay to allow its stream, as the
/// gatherer waits. Provisional.
const OPEN_WAIT: Duration = Duration::from_secs(10);

/// Events of the relay's unidirectional streams not taken yet; as the
/// gatherer's. Provisional.
const UNI_QUEUE: usize = 64;

/// Requests the relay opened that are not taken yet. Provisional.
const PEER_REQUEST_QUEUE: usize = 16;

/// What the tasks of the request streams said and the loop did not take
/// yet. Provisional.
const HEARD_QUEUE: usize = 256;

/// Pieces of a transcript that wait for its `PUBLISH_OK`; past it a piece is
/// not sent live, and a FETCH gets it. Provisional.
const PIECES_WAITING_FOR_A_PUBLISH: usize = 64;

/// Subgroup streams of pieces being written at once; past it a piece is not
/// sent live. Provisional.
const PIECES_BEING_SENT: usize = 64;

/// FETCH requests answered at once, as the gatherer answers. Provisional.
const FETCHES_AT_ONCE: usize = 32;

/// The bytes of payload the FETCH answers being written hold together
/// ([`Held`]): 16 answers of the default 1,000 pieces of 1 KB. Provisional.
const MAX_BYTES_HELD: usize = 16 * 1000 * 1024;

/// How long a transcript whose PUBLISH failed waits before it is published
/// again. Provisional.
const PUBLISH_AGAIN_AFTER: Duration = Duration::from_secs(5);

/// The probability of a voice from which a frame has one: whisper.cpp's
/// default (`whisper_vad_default_params`). Provisional.
const VAD_THRESHOLD: f32 = 0.5;

/// The name of the catalog track (MSF §5).
const CATALOG: &[u8] = b"catalog";

/// The most a catalog may be, as the terminal viewer reads it.
const CATALOG_LIMIT: usize = 1024 * 1024;

/// The Track Alias of the first transcript published; each next one has the
/// next (§11.1).
const FIRST_TRACK_ALIAS: u64 = 1;

/// How a session ended without an error.
#[derive(Debug)]
pub enum Ended {
    /// The transcriber was told to stop.
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

/// What a request of the transcriber is for.
#[derive(Debug, Clone)]
enum Purpose {
    /// A `SUBSCRIBE_NAMESPACE` for a prefix, by its place in the flags.
    Watch(usize),
    /// The SUBSCRIBE for the catalog of a broadcast.
    Catalog(TrackNamespace),
    /// The Joining FETCH for the catalog as it is: one Object that was
    /// published before the transcriber came (§10.12.2), as the terminal
    /// viewer reads it.
    CatalogFetch(TrackNamespace),
    /// The SUBSCRIBE for the audio track of a broadcast.
    Audio(TrackNamespace),
    /// The PUBLISH of the transcript of a prefix.
    Speech(usize),
}

/// A request of the transcriber, by the token its task reports with.
struct Own {
    purpose: Purpose,
    request: RequestId,
    task: RequestTask,
}

/// One `SUBSCRIBE_NAMESPACE`, and when it is asked for again after it
/// ended.
struct Watch {
    prefix: TrackNamespace,
    again: Option<Instant>,
    sent: Instant,
    backoff: Backoff,
}

/// How an audio subscription ended.
#[derive(Debug, Clone, Copy)]
enum Over {
    /// It ended, or was not answered.
    Ended,
    /// It was refused, with the Retry Interval the refusal names, if any.
    Refused(Option<u64>),
}

/// A broadcast that is listened to.
struct Broadcast {
    /// The prefix it is under.
    prefix: usize,
    /// Its namespace, each field as UTF-8: `from` of its pieces.
    from: Vec<String>,
    catalog: Option<u64>,
    catalog_fetch: Option<u64>,
    /// The name of the audio track the catalog names.
    audio_name: Option<Vec<u8>>,
    /// The token of its SUBSCRIBE, while it has one.
    audio: Option<u64>,
    listener: Listener,
    again: AskAgain,
}

/// A PUBLISH of a transcript that is to end once its streams are closed
/// (§10.11), as the gatherer ends a room's.
struct Ending {
    prefix: usize,
    status: u64,
    opened: Arc<AtomicU64>,
}

/// The PUBLISH of the transcript of one prefix.
struct Published {
    publish: Option<u64>,
    track_alias: u64,
    live: bool,
    opened: Arc<AtomicU64>,
    waiting: VecDeque<Speech>,
    not_before: Option<Instant>,
    sending: watch::Sender<usize>,
}

impl Published {
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

/// A FETCH of the relay that the transcriber answers, as the gatherer's.
struct Fetching {
    request: PeerRequestId,
    answering: Answering,
    over: bool,
    peer_finished: bool,
    bytes: usize,
}

/// Everything of one session.
struct Transcribing<'a> {
    connection: &'a Arc<WtConnection>,
    args: &'a Args,
    reader: &'a mut Reader,
    transcripts: &'a mut [Transcript],
    key: Option<&'a RoomKey>,
    /// The model of voice activity; `None` with `--voice loudness`.
    vad: Option<&'a Path>,
    session: Session,
    stops: StreamStops,
    own: HashMap<u64, Own>,
    next_token: u64,
    heard: mpsc::Sender<(u64, Heard)>,
    published: Vec<Published>,
    next_track_alias: u64,
    watches: Vec<Watch>,
    broadcasts: HashMap<TrackNamespace, Broadcast>,
    /// The broadcasts whose audio is to be asked for again, and when.
    asking_again: HashMap<TrackNamespace, Instant>,
    /// The broadcast each catalog FETCH is for, by its Request ID.
    catalog_fetches: HashMap<u64, TrackNamespace>,
    catalogs: mpsc::Sender<(TrackNamespace, RequestId, Option<Vec<u8>>)>,
    /// Objects read before the `SUBSCRIBE_OK` that gives their Track Alias,
    /// by that alias (`xmoq_moqt::early`).
    early: EarlyObjects<(u64, SubgroupObject)>,
    refused: HashMap<u64, PeerRequestId>,
    fetches: HashMap<u64, Fetching>,
    fetch_done: mpsc::Sender<(u64, Answered)>,
    held: Held,
    ending: HashMap<u64, Ending>,
    drained: mpsc::Sender<u64>,
    sends: JoinSet<()>,
    replies: JoinSet<()>,
    /// Audio Objects without a Timestamp of LOC, which are not read.
    untimed: u64,
}

/// What the session says the peer did wrong, as an error that carries its
/// code.
fn fault(fault: RequestFault) -> anyhow::Error {
    match fault {
        RequestFault::Peer(error) => error.into(),
        RequestFault::Usage(error) => anyhow!("the transcriber used its session wrongly: {error}"),
    }
}

/// The fields of a namespace as UTF-8; `None` where one is not.
fn fields_of(namespace: &TrackNamespace) -> Option<Vec<String>> {
    namespace
        .fields()
        .iter()
        .map(|field| String::from_utf8(field.clone()).ok())
        .collect()
}

/// Reads a catalog's fetch stream to its end: the last Object, which is the
/// catalog as it is now.
async fn last_object(
    header: xmoq_moqt::fetch::FetchHeader,
    mut body: xmoq_moqt_io::FetchBody,
) -> Option<Vec<u8>> {
    let mut decoder = FetchStreamDecoder::new(CATALOG_LIMIT, GroupOrder::Ascending);
    // The readers took the header off; the decoder wants to see one.
    decoder
        .push(&FetchStreamEncoder::new(header, GroupOrder::Ascending).take_bytes())
        .ok()?;
    let mut latest = None;
    loop {
        while let Some(event) = decoder.next_event().ok()? {
            if let FetchEvent::Object(object) = event {
                latest = Some(object.payload);
            }
        }
        match body.next().await {
            FetchChunk::Bytes(bytes) => decoder.push(&bytes).ok()?,
            FetchChunk::End(_) | FetchChunk::Stopped => return latest,
        }
    }
}

/// Runs one session on `connection` until it ends.
pub async fn transcribe(
    connection: &Arc<WtConnection>,
    args: &Args,
    (reader, transcripts, key, vad): (
        &mut Reader,
        &mut [Transcript],
        Option<&RoomKey>,
        Option<&Path>,
    ),
    read: &mut mpsc::Receiver<Read>,
    stop: &mut watch::Receiver<bool>,
) -> Result<Ended> {
    let stops = StreamStops::default();
    let (uni_queue, mut uni) = mpsc::channel(UNI_QUEUE);
    let stream_config = StreamConfig::default();
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
    let (fetch_done, mut fetches_done) = mpsc::channel(FETCHES_AT_ONCE);
    let (drained, mut drained_publishes) = mpsc::channel(PEER_REQUEST_QUEUE);
    let (catalogs, mut catalogs_read) = mpsc::channel(PEER_REQUEST_QUEUE);
    let mut transcribing = Transcribing {
        connection,
        args,
        reader,
        transcripts,
        key,
        vad,
        session: Session::new(Role::Client, Transport::WebTransport),
        stops,
        own: HashMap::new(),
        next_token: 0,
        heard,
        published: args.prefixes.iter().map(|_| Published::new()).collect(),
        next_track_alias: FIRST_TRACK_ALIAS,
        watches: args
            .prefixes
            .iter()
            .map(|prefix| Watch {
                prefix: prefix.clone(),
                again: None,
                sent: Instant::now(),
                backoff: Backoff::default(),
            })
            .collect(),
        broadcasts: HashMap::new(),
        asking_again: HashMap::new(),
        catalog_fetches: HashMap::new(),
        catalogs,
        early: EarlyObjects::default(),
        refused: HashMap::new(),
        fetches: HashMap::new(),
        fetch_done,
        held: Held::new(MAX_BYTES_HELD),
        ending: HashMap::new(),
        drained,
        sends: JoinSet::new(),
        replies: JoinSet::new(),
        untimed: 0,
    };

    // §3.3: one control stream that begins with SETUP.
    let mut control = connection.open_uni().await?;
    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    // In ascending order of their types: the token (0x03), then the
    // implementation (0x07). The relay looks at no namespace or role of the
    // token of a SETUP.
    let first_prefix = args.prefixes.first().cloned();
    let mut options: Vec<KeyValuePair> = first_prefix
        .and_then(|prefix| {
            transcribing.token(&format!("{}/*", namespace_text(&prefix)), TokenRole::Sub)
        })
        .map(|token| KeyValuePair::bytes(setup_option::AUTHORIZATION_TOKEN, token))
        .transpose()?
        .into_iter()
        .collect();
    options.push(KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        implementation.as_bytes().to_vec(),
    )?);
    let setup = transcribing.session.setup(options)?;
    send_message(&mut control, &setup, &Trace::default(), "control").await?;
    match tokio::time::timeout(SETUP_WAIT, uni.recv()).await {
        Ok(Some(UniEvent::Control(message))) => {
            transcribing.session.on_control_message(message)?;
        }
        Ok(Some(UniEvent::Failed(error))) => return Err(error.into()),
        Ok(other) => bail!("no SETUP from the relay: {other:?}"),
        Err(_) => return Err(NoSetupInTime.into()),
    }
    println!("SETUP_OK");
    for index in 0..transcribing.watches.len() {
        transcribing.watch(index)?;
        transcribing.publish(index)?;
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
                if let Some(ended) = transcribing.on_uni(event)? {
                    break ended;
                }
            }
            event = peer.recv() => {
                let event = event.ok_or_else(|| anyhow!("the connection is gone"))?;
                transcribing.on_peer_request(event)?;
            }
            Some((token, heard)) = heard_by_loop.recv() => {
                if let Some(ended) = transcribing.on_heard(token, heard)? {
                    break ended;
                }
            }
            Some(piece) = read.recv() => transcribing.on_read(piece)?,
            Some((namespace, request, payload)) = catalogs_read.recv() => {
                transcribing.session.on_fetch_stream_closed(request);
                transcribing.catalog_fetched(&namespace);
                if let Some(payload) = payload {
                    transcribing.on_catalog(&namespace, &payload)?;
                }
            }
            Some((stream, answered)) = fetches_done.recv() => transcribing.fetch_over(stream, answered),
            Some(token) = drained_publishes.recv() => transcribing.publish_done(token)?,
            () = sleep_until(transcribing.next_watch()) => transcribing.watch_again()?,
            () = sleep_until(transcribing.next_audio()) => transcribing.audio_again()?,
            Some(_) = transcribing.sends.join_next() => {}
            Some(_) = transcribing.replies.join_next() => {}
        }
    };
    // The pieces under way are read: what was said before the end is not
    // lost to it.
    let namespaces: Vec<TrackNamespace> = transcribing.broadcasts.keys().cloned().collect();
    for namespace in namespaces {
        transcribing.listen_no_more(&namespace);
    }
    drop(control);
    Ok(ended)
}

/// A namespace as the tokens of the room write it: its fields between
/// slashes.
fn namespace_text(namespace: &TrackNamespace) -> String {
    let fields: Vec<String> = namespace
        .fields()
        .iter()
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect();
    fields.join("/")
}

impl Transcribing<'_> {
    /// A token of the transcriber's own for `role` in `namespace` that runs
    /// for [`TOKEN_SECONDS`], as an AUTHORIZATION TOKEN (§10.2.2); `None`
    /// without `--auth-key-hex`. Made anew for each request, as the
    /// gatherer's (why, at its `token`).
    fn token(&self, namespace: &str, role: TokenRole) -> Option<Vec<u8>> {
        let key = self.key?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let text = key.make(namespace, role, now.saturating_add(TOKEN_SECONDS));
        Some(
            Token::UseValue {
                token_type: 0,
                value: text.into_bytes(),
            }
            .encode(),
        )
    }

    /// The token to read what is under a prefix: `sub` in `<prefix>/*`.
    fn sub_token(&self, prefix: usize) -> Option<Parameter> {
        let prefix = self.args.prefixes.get(prefix)?;
        self.token(&format!("{}/*", namespace_text(prefix)), TokenRole::Sub)
            .map(Parameter::AuthorizationToken)
    }

    /// Asks for the namespaces under a prefix.
    fn watch(&mut self, index: usize) -> Result<()> {
        let parameters: Vec<Parameter> = self.sub_token(index).into_iter().collect();
        let Some(watch) = self.watches.get_mut(index) else {
            return Ok(());
        };
        watch.again = None;
        watch.sent = Instant::now();
        let prefix = watch.prefix.clone();
        let (request, message) = self.session.subscribe_namespace(prefix, parameters)?;
        self.open(Purpose::Watch(index), request, message, &[]);
        Ok(())
    }

    fn next_watch(&self) -> Option<Instant> {
        self.watches.iter().filter_map(|watch| watch.again).min()
    }

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

    /// Opens the stream of a request.
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

    /// Tells the task of a request stream something; where its queue is
    /// full the request is cancelled, as the gatherer does (why, at its
    /// `tell`).
    fn tell(&mut self, token: u64, command: Command) {
        let full = self
            .own
            .get(&token)
            .is_some_and(|own| own.task.commands.try_send(command).is_err());
        if full
            && let Some(purpose) =
                self.let_go(token, Command::Cancel(stream_reset_code::EXCESSIVE_LOAD))
        {
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
            command => {
                let _ = own.task.commands.try_send(command);
            }
        }
        Some(own.purpose)
    }

    /// Publishes the transcript of a prefix, unless it is published or waits
    /// to be published again.
    fn publish(&mut self, prefix: usize) -> Result<()> {
        let Some(listened) = self.args.prefixes.get(prefix).cloned() else {
            return Ok(());
        };
        let Some(entry) = self.published.get(prefix) else {
            return Ok(());
        };
        if entry.publish.is_some() || entry.not_before.is_some_and(|after| Instant::now() < after) {
            return Ok(());
        }
        let track_alias = self.next_track_alias;
        self.next_track_alias += 1;
        let pub_token = self
            .token(
                &namespace_text(&transcript_namespace(&listened)?),
                TokenRole::Pub,
            )
            .map(Parameter::AuthorizationToken);
        // §10.2.11: the latest piece, which the relay gives its subscribers
        // in SUBSCRIBE_OK, and where a Joining FETCH ends.
        let parameters: Vec<Parameter> = self
            .transcripts
            .get(prefix)
            .and_then(Transcript::largest)
            .map(|group| Parameter::LargestObject(Location { group, object: 0 }))
            .into_iter()
            .chain(pub_token)
            .collect();
        let published = self.session.publish(
            speech_track(&listened)?,
            track_alias,
            parameters.clone(),
            vec![],
        );
        let Ok((request, message)) = published else {
            println!(
                "TRANSCRIPT_NOT_PUBLISHED prefix={} why=\"the PUBLISH before is ending\"",
                shown(&listened)
            );
            return Ok(());
        };
        let token = self.open(Purpose::Speech(prefix), request, message, &parameters);
        if let Some(entry) = self.published.get_mut(prefix) {
            entry.publish = Some(token);
            entry.track_alias = track_alias;
            entry.live = false;
            entry.opened = Arc::new(AtomicU64::new(0));
            entry.not_before = None;
        }
        Ok(())
    }

    /// The PUBLISH of a transcript is over; it is published again after a
    /// pause, on the next piece.
    fn unpublish(&mut self, prefix: usize, token: u64) {
        if let Some(entry) = self.published.get_mut(prefix)
            && entry.publish == Some(token)
        {
            entry.publish = None;
            entry.live = false;
            entry.waiting.clear();
            entry.not_before = Some(Instant::now() + PUBLISH_AGAIN_AFTER);
            println!("TRANSCRIPT_UNPUBLISHED prefix={prefix}");
        }
    }

    /// Ends the PUBLISH of a transcript with `status` once its streams being
    /// sent are closed ([`Self::publish_done`]), as the gatherer ends a
    /// room's.
    fn end_publish(&mut self, prefix: usize, status: u64) {
        let Some(entry) = self.published.get_mut(prefix) else {
            return;
        };
        let Some(token) = entry.publish.take() else {
            return;
        };
        entry.live = false;
        entry.waiting.clear();
        let sending = entry.sending.subscribe();
        let opened = Arc::clone(&entry.opened);
        self.ending.insert(
            token,
            Ending {
                prefix,
                status,
                opened,
            },
        );
        let drained = self.drained.clone();
        tokio::spawn(async move {
            let mut sending = sending;
            let _ = sending.wait_for(|count| *count == 0).await;
            let _ = drained.send(token).await;
        });
    }

    /// Every stream of an ending PUBLISH is closed: its `PUBLISH_DONE` with
    /// the number of streams (§10.11), and the transcript is published anew.
    fn publish_done(&mut self, token: u64) -> Result<()> {
        let Some(ending) = self.ending.remove(&token) else {
            return Ok(());
        };
        if let Some(own) = self.own.get(&token) {
            let streams = ending.opened.load(Ordering::Relaxed);
            let done = self.session.publish_done(
                own.request,
                ending.status,
                streams,
                ReasonPhrase::default(),
            )?;
            self.tell(token, Command::Say(done));
            self.let_go(token, Command::Finish);
            println!(
                "TRANSCRIPT_DONE prefix={} status={:#x} streams={streams}",
                ending.prefix, ending.status
            );
        }
        self.publish(ending.prefix)
    }

    /// The pieces of a prefix's broadcasts that were read.
    fn on_read(&mut self, read: Read) -> Result<()> {
        let Some(transcript) = self.transcripts.get_mut(read.prefix) else {
            return Ok(());
        };
        let speech = transcript.add(read.said);
        // On the clock of the broadcast's Timestamps, which are the
        // publisher's wall clock in microseconds for the demo's pages: how
        // long after the end of the piece its words were there.
        let behind_ms = i128::from(speech.at) - i128::from(speech.end_us / 1000);
        println!(
            "SPEECH prefix={} number={} from={} start_us={} end_us={} waited_ms={} took_ms={} behind_ms={behind_ms} text={:?}",
            read.prefix,
            speech.number,
            speech.from.join("/"),
            speech.start_us,
            speech.end_us,
            read.waited_ms,
            read.took_ms,
            speech.text
        );
        self.send_live(read.prefix, speech)
    }

    /// Sends a piece on the transcript of its prefix, or has it wait for the
    /// `PUBLISH_OK`.
    fn send_live(&mut self, prefix: usize, speech: Speech) -> Result<()> {
        let is_live = self.published.get(prefix).is_some_and(|entry| entry.live);
        if !is_live {
            self.publish(prefix)?;
            if let Some(entry) = self.published.get_mut(prefix)
                && entry.publish.is_some()
            {
                if entry.waiting.len() >= PIECES_WAITING_FOR_A_PUBLISH {
                    entry.waiting.pop_front();
                }
                entry.waiting.push_back(speech);
            }
            return Ok(());
        }
        let Some(entry) = self.published.get(prefix) else {
            return Ok(());
        };
        if self.sends.len() >= PIECES_BEING_SENT {
            println!(
                "SPEECH_NOT_SENT prefix={prefix} number={} why=too-many-at-once",
                speech.number
            );
            return Ok(());
        }
        let group = GroupToSend {
            track_alias: entry.track_alias,
            group_id: speech.number,
            payload: speech.payload(),
            opened: Arc::clone(&entry.opened),
        };
        let counted = Sending::begin(&entry.sending);
        let connection = Arc::clone(self.connection);
        self.sends.spawn(async move {
            let _counted = counted;
            let number = group.group_id;
            if let Err(why) = send_group(connection, group).await {
                println!("SPEECH_NOT_SENT prefix={prefix} number={number} why={why:?}");
            }
        });
        Ok(())
    }

    /// A NAMESPACE under a prefix: a broadcast, whose catalog is asked for.
    fn on_namespace(&mut self, prefix: usize, namespace: &TrackNamespace) -> Result<()> {
        if self.broadcasts.contains_key(namespace) {
            return Ok(());
        }
        // WHY UTF-8: the fields go into the JSON of a piece as strings.
        let Some(from) = fields_of(namespace) else {
            println!(
                "BROADCAST_IGNORED namespace={} why=not-utf-8",
                shown(namespace)
            );
            return Ok(());
        };
        if self.broadcasts.len() >= self.args.tracks {
            println!(
                "BROADCAST_IGNORED namespace={} why=too-many",
                shown(namespace)
            );
            return Ok(());
        }
        println!("BROADCAST namespace={}", shown(namespace));
        self.broadcasts.insert(
            namespace.clone(),
            Broadcast {
                prefix,
                from,
                catalog: None,
                catalog_fetch: None,
                audio_name: None,
                audio: None,
                listener: Listener::new(self.judge()?, Cut::default())?,
                again: AskAgain::default(),
            },
        );
        let parameters = self.subscribe_parameters(prefix);
        let track = FullTrackName::new(namespace.clone(), CATALOG.to_vec())?;
        let Ok((request, message)) = self.session.subscribe(track, parameters.clone()) else {
            return Ok(());
        };
        let token = self.open(
            Purpose::Catalog(namespace.clone()),
            request,
            message,
            &parameters,
        );
        if let Some(broadcast) = self.broadcasts.get_mut(namespace) {
            broadcast.catalog = Some(token);
        }
        Ok(())
    }

    /// How a broadcast's voice is told: by a model of voice activity of its
    /// own, which holds what it heard of that broadcast alone.
    fn judge(&self) -> Result<Judge> {
        Ok(match self.vad {
            Some(model) => Judge::Model(ModelJudge::new(
                Box::new(Silero::open(model)?),
                VAD_THRESHOLD,
            )),
            None => Judge::Loudness(VOICE_DBFS),
        })
    }

    /// The parameters of a SUBSCRIBE under a prefix. Without a filter, as
    /// the terminal viewer subscribes: the catalog that is there comes on
    /// it, and the sound from now on.
    fn subscribe_parameters(&self, prefix: usize) -> Vec<Parameter> {
        let mut parameters = vec![
            Parameter::Forward(true),
            Parameter::SubscriberPriority(128),
            Parameter::GroupOrder(GroupOrder::Ascending),
        ];
        parameters.extend(self.sub_token(prefix));
        parameters
    }

    /// The catalog of a broadcast: its audio track is listened to, or the one
    /// it names now in place of another.
    fn on_catalog(&mut self, namespace: &TrackNamespace, payload: &[u8]) -> Result<()> {
        let tracks = match xmoq_msf::read_catalog(payload) {
            Ok(tracks) => tracks,
            Err(error) => {
                println!(
                    "CATALOG_UNREAD namespace={} error={error}",
                    shown(namespace)
                );
                return Ok(());
            }
        };
        let Some(name) = xmoq_msf::first_audio_track(&tracks, "opus")
            .map(|track| track.name.clone().into_bytes())
        else {
            println!("CATALOG_WITHOUT_AUDIO namespace={}", shown(namespace));
            return Ok(());
        };
        let Some(broadcast) = self.broadcasts.get_mut(namespace) else {
            return Ok(());
        };
        if broadcast.audio_name.as_ref() == Some(&name) {
            return Ok(());
        }
        broadcast.audio_name = Some(name);
        let old = broadcast.audio.take();
        if let Some(token) = old {
            self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
            self.end_piece(namespace);
        }
        self.subscribe_audio(namespace)
    }

    /// Subscribes to the audio track of a broadcast.
    fn subscribe_audio(&mut self, namespace: &TrackNamespace) -> Result<()> {
        self.asking_again.remove(namespace);
        let Some(broadcast) = self.broadcasts.get(namespace) else {
            return Ok(());
        };
        let (Some(name), None) = (broadcast.audio_name.clone(), broadcast.audio) else {
            return Ok(());
        };
        let parameters = self.subscribe_parameters(broadcast.prefix);
        let track = FullTrackName::new(namespace.clone(), name)?;
        let Ok((request, message)) = self.session.subscribe(track, parameters.clone()) else {
            return Ok(());
        };
        let token = self.open(
            Purpose::Audio(namespace.clone()),
            request,
            message,
            &parameters,
        );
        if let Some(broadcast) = self.broadcasts.get_mut(namespace) {
            broadcast.audio = Some(token);
        }
        Ok(())
    }

    /// The audio subscription of a broadcast that stays announced is over:
    /// the piece under way is read, and the track is asked for again after
    /// a pause by the rule every client asks by (`xmoq-again`).
    ///
    /// WHY no count of times, as the gatherer has for a participant's `said`:
    /// a broadcast without sound names no audio track in its catalog, and
    /// is not asked; one that names it is meant to be heard.
    fn audio_over(&mut self, namespace: &TrackNamespace, over: Over, why: &str) {
        println!(
            "AUDIO_NOT_SUBSCRIBED namespace={} why={why:?}",
            shown(namespace)
        );
        self.end_piece(namespace);
        let Some(broadcast) = self.broadcasts.get_mut(namespace) else {
            return;
        };
        broadcast.audio = None;
        let pause = match over {
            Over::Refused(named) => broadcast.again.after_refusal(named),
            Over::Ended => broadcast.again.after_end(),
        };
        println!("ASKING_AGAIN namespace={} in_ms={pause}", shown(namespace));
        self.asking_again.insert(
            namespace.clone(),
            Instant::now() + Duration::from_millis(pause),
        );
    }

    fn next_audio(&self) -> Option<Instant> {
        self.asking_again.values().min().copied()
    }

    fn audio_again(&mut self) -> Result<()> {
        let now = Instant::now();
        let due: Vec<TrackNamespace> = self
            .asking_again
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(namespace, _)| namespace.clone())
            .collect();
        for namespace in due {
            self.subscribe_audio(&namespace)?;
        }
        Ok(())
    }

    /// The piece under way of a broadcast goes to the reader.
    fn end_piece(&mut self, namespace: &TrackNamespace) {
        let Some(broadcast) = self.broadcasts.get_mut(namespace) else {
            return;
        };
        let (prefix, from) = (broadcast.prefix, broadcast.from.clone());
        for piece in broadcast.listener.end() {
            self.give(ToRead {
                prefix,
                from: from.clone(),
                piece,
            });
        }
    }

    /// Gives a piece to the reader, and says where it was dropped.
    fn give(&mut self, to_read: ToRead) {
        let (from, start_us) = (to_read.from.join("/"), to_read.piece.start_us);
        if let Some(dropped) = self.reader.give(to_read) {
            println!("PIECE_DROPPED from={from} start_us={start_us} dropped={dropped}");
        }
    }

    /// A broadcast is listened to no more: its requests are let go of, and
    /// its piece under way is read.
    fn listen_no_more(&mut self, namespace: &TrackNamespace) {
        self.end_piece(namespace);
        self.asking_again.remove(namespace);
        let Some(broadcast) = self.broadcasts.remove(namespace) else {
            return;
        };
        for token in [broadcast.catalog, broadcast.catalog_fetch, broadcast.audio]
            .into_iter()
            .flatten()
        {
            self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
        }
        self.catalog_fetches
            .retain(|_, fetched| fetched != namespace);
    }

    /// An Object of the audio track of a broadcast.
    fn on_audio(&mut self, namespace: &TrackNamespace, object: &SubgroupObject) {
        let properties = searched_integers(&object.properties, Of::Object);
        let Some(timestamp_us) = loc::wall_clock_timestamp(properties) else {
            self.untimed += 1;
            if self.untimed.is_power_of_two() {
                println!(
                    "AUDIO_WITHOUT_TIMESTAMP namespace={} count={}",
                    shown(namespace),
                    self.untimed
                );
            }
            return;
        };
        let Some(broadcast) = self.broadcasts.get_mut(namespace) else {
            return;
        };
        let pieces = broadcast.listener.push(AudioPacket {
            timestamp_us,
            opus: &object.payload,
        });
        let (prefix, from) = (broadcast.prefix, broadcast.from.clone());
        for piece in pieces {
            println!(
                "PIECE_CUT from={} start_us={} end_us={}",
                from.join("/"),
                piece.start_us,
                piece.end_us
            );
            self.give(ToRead {
                prefix,
                from: from.clone(),
                piece,
            });
        }
    }

    /// An Object of a track the transcriber subscribed to.
    fn on_object(&mut self, track: &FullTrackName, object: &SubgroupObject) -> Result<()> {
        let namespace = track.namespace();
        let Some(broadcast) = self.broadcasts.get(namespace) else {
            return Ok(());
        };
        if track.name() == CATALOG {
            return self.on_catalog(&namespace.clone(), &object.payload);
        }
        if broadcast.audio_name.as_deref() == Some(track.name()) {
            self.on_audio(&namespace.clone(), object);
        }
        Ok(())
    }

    /// An event of the relay's unidirectional streams.
    fn on_uni(&mut self, event: UniEvent) -> Result<Option<Ended>> {
        // What a SUBSCRIBE that ended meanwhile left held is let go of before
        // what came after it.
        self.take_early()?;
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
                let object =
                    match self
                        .early
                        .arrive(&self.session, stream, alias, bytes, (alias, object))
                    {
                        Arrived::Deliver((_, object)) => object,
                        Arrived::Held => return Ok(None),
                        Arrived::Stop { stream, code } => {
                            self.stops.stop(stream, code);
                            return Ok(None);
                        }
                    };
                let track = match self.session.subscribed_alias(alias) {
                    SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => {
                        track.clone()
                    }
                    SubscribedAlias::Unknown | SubscribedAlias::Retired => return Ok(None),
                };
                self.on_object(&track, &object)?;
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
            UniEvent::FetchOpened { stream, header } => {
                let body = self.stops.take_fetch_body(stream);
                let FetchStreamOpened::Fetch(request) =
                    self.session.on_fetch_stream(header.request_id)?
                else {
                    drop(body);
                    return Ok(None);
                };
                let (Some(namespace), Some(body)) =
                    (self.catalog_fetches.get(&header.request_id).cloned(), body)
                else {
                    self.session.on_fetch_stream_closed(request);
                    return Ok(None);
                };
                let catalogs = self.catalogs.clone();
                tokio::spawn(async move {
                    let payload = last_object(header, body).await;
                    let _ = catalogs.send((namespace, request, payload)).await;
                });
            }
            UniEvent::SubgroupOpened { .. } | UniEvent::ObjectBegun { .. } => {}
            UniEvent::Failed(error) => return Err(error.into()),
            UniEvent::ReaderPanicked => bail!("a reader of a stream panicked"),
        }
        Ok(None)
    }

    /// What the task of one of the transcriber's request streams heard.
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
            // PUBLISH's stream has not ended the subscription; nor has a
            // publisher that finishes its direction of a FETCH it answered.
            Heard::Closed(RequestClose::Fin)
                if matches!(purpose, Purpose::Speech(_) | Purpose::CatalogFetch(_)) =>
            {
                Ok(None)
            }
            Heard::Closed(how) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, &purpose, &format!("its stream ended ({how})"));
                Ok(None)
            }
            Heard::Stopped(code) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, &purpose, &format!("stopped ({code:?})"));
                Ok(None)
            }
            Heard::Failed(RequestReadError::Peer(error)) => Err(error.into()),
            Heard::Failed(RequestReadError::Lost) => bail!("the connection is gone"),
        }
    }

    /// A request of the transcriber is over, and was let go of.
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
            // WHY the broadcast goes, and not a catalog asked for again: its
            // NAMESPACE comes again with the next SUBSCRIBE_NAMESPACE, and a
            // broadcast without a catalog has no track to listen to.
            Purpose::Catalog(namespace) => {
                println!(
                    "CATALOG_NOT_SUBSCRIBED namespace={} why={why:?}",
                    shown(namespace)
                );
                if let Some(broadcast) = self.broadcasts.get_mut(namespace) {
                    broadcast.catalog = None;
                }
            }
            Purpose::CatalogFetch(namespace) => {
                if let Some(broadcast) = self.broadcasts.get_mut(namespace) {
                    broadcast.catalog_fetch = None;
                }
            }
            Purpose::Audio(namespace) => self.audio_over(&namespace.clone(), Over::Ended, why),
            Purpose::Speech(prefix) => {
                println!("TRANSCRIPT_NOT_PUBLISHED prefix={prefix} why={why:?}");
                self.unpublish(*prefix, token);
                self.ending.remove(&token);
            }
        }
    }

    /// The catalog's SUBSCRIBE was answered: the catalog as it is is
    /// fetched.
    fn catalog_established(&mut self, namespace: &TrackNamespace, track_alias: u64) -> Result<()> {
        // The catalog's Objects are held to its limit, and not to what every
        // subgroup stream may hold alone.
        self.stops.limit_payload(track_alias, CATALOG_LIMIT);
        let Some(catalog) = self
            .broadcasts
            .get(namespace)
            .and_then(|broadcast| broadcast.catalog)
        else {
            return Ok(());
        };
        let Some(own) = self.own.get(&catalog) else {
            return Ok(());
        };
        let range = FetchRange::RelativeJoining {
            joining_request_id: own.request.value(),
            joining_start: 0,
        };
        let prefix = self
            .broadcasts
            .get(namespace)
            .map_or(0, |broadcast| broadcast.prefix);
        let parameters: Vec<Parameter> = self.sub_token(prefix).into_iter().collect();
        let (request, message) = self.session.fetch(range, parameters.clone())?;
        self.catalog_fetches
            .insert(request.value(), namespace.clone());
        let token = self.open(
            Purpose::CatalogFetch(namespace.clone()),
            request,
            message,
            &parameters,
        );
        if let Some(broadcast) = self.broadcasts.get_mut(namespace) {
            broadcast.catalog_fetch = Some(token);
        }
        self.take_early()
    }

    /// The fetch stream of a broadcast's catalog was read: its request is
    /// finished, and the relay's direction is waited for
    /// (`xmoq_track_serving::streams::PEER_END_WAIT`). WHY: a request left
    /// open for each broadcast stays in the session, which holds 256 at the
    /// most.
    fn catalog_fetched(&mut self, namespace: &TrackNamespace) {
        self.catalog_fetches
            .retain(|_, fetched| fetched != namespace);
        let token = self
            .broadcasts
            .get_mut(namespace)
            .and_then(|broadcast| broadcast.catalog_fetch.take());
        if let Some(token) = token {
            self.let_go(token, Command::Finish);
        }
    }

    /// The Objects held for an alias the session knows now are taken, in the
    /// order they came, and the streams of those no `SUBSCRIBE` can give any
    /// more are stopped (`xmoq_moqt::early`).
    fn take_early(&mut self) -> Result<()> {
        if self.early.is_empty() {
            return Ok(());
        }
        let settled = self.early.settle(&self.session);
        for (stream, code) in settled.stopped {
            self.stops.stop(stream, code);
        }
        for (track_alias, object) in settled.released {
            let track = match self.session.subscribed_alias(track_alias) {
                SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => {
                    track.clone()
                }
                SubscribedAlias::Unknown | SubscribedAlias::Retired => continue,
            };
            self.on_object(&track, &object)?;
        }
        Ok(())
    }

    /// The `PUBLISH_OK` of a transcript: the pieces that waited for it are
    /// sent.
    fn published_ok(&mut self, prefix: usize, token: u64, forward: bool) -> Result<()> {
        let waiting = match self.published.get_mut(prefix) {
            // Not for a PUBLISH that is ending, or one before.
            Some(entry) if entry.publish == Some(token) => {
                entry.live = forward;
                std::mem::take(&mut entry.waiting)
            }
            _ => VecDeque::new(),
        };
        let largest = self.transcripts.get(prefix).and_then(Transcript::largest);
        println!(
            "TRANSCRIPT_PUBLISHED prefix={} forward={forward} largest={}",
            self.args
                .prefixes
                .get(prefix)
                .map_or_else(String::new, shown),
            largest.map_or_else(|| "none".to_string(), |largest| largest.to_string())
        );
        for speech in waiting {
            self.send_live(prefix, speech)?;
        }
        Ok(())
    }

    /// The meaning of a message on a request stream of the transcriber.
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
            (Purpose::Watch(index), RequestEvent::Namespace(namespace)) => {
                self.on_namespace(*index, &namespace)?;
            }
            (Purpose::Watch(_), RequestEvent::NamespaceDone(namespace)) => {
                println!("BROADCAST_GONE namespace={}", shown(&namespace));
                self.listen_no_more(&namespace);
            }
            (
                Purpose::Catalog(namespace),
                RequestEvent::SubscribeEstablished { track_alias, .. },
            ) => {
                self.catalog_established(&namespace.clone(), track_alias)?;
            }
            (Purpose::Audio(namespace), RequestEvent::SubscribeEstablished { .. }) => {
                println!("AUDIO_SUBSCRIBED namespace={}", shown(namespace));
                if let Some(broadcast) = self.broadcasts.get_mut(namespace) {
                    broadcast.again.subscribed();
                }
                self.take_early()?;
            }
            (Purpose::Speech(prefix), RequestEvent::PublishEstablished { forward, .. }) => {
                self.published_ok(*prefix, token, forward)?;
            }
            (Purpose::Speech(_), RequestEvent::SubscriptionUpdated { response, .. }) => {
                self.tell(token, Command::Say(response));
            }
            // §10.9.1: the update is refused, and the subscription ends with
            // UPDATE_FAILED.
            (Purpose::Speech(prefix), RequestEvent::UpdateRefused { response }) => {
                let prefix = *prefix;
                self.tell(token, Command::Say(response));
                if self
                    .published
                    .get(prefix)
                    .is_some_and(|entry| entry.publish == Some(token))
                {
                    self.end_publish(prefix, publish_done_status::UPDATE_FAILED);
                }
            }
            (Purpose::Audio(namespace), RequestEvent::Rejected(error)) => {
                let namespace = namespace.clone();
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                self.audio_over(&namespace, Over::Refused(error.retry_wait_ms()), &why);
            }
            (_, RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                let why = format!("refused with {:#x}", error.error_code);
                self.request_over(token, purpose, &why);
            }
            (Purpose::Audio(namespace), RequestEvent::SubscriptionDone(done)) => {
                let namespace = namespace.clone();
                self.let_go(token, Command::Finish);
                let why = format!("PUBLISH_DONE {:#x}", done.status_code);
                self.audio_over(&namespace, Over::Ended, &why);
            }
            (_, RequestEvent::SubscriptionDone(_)) => {
                self.let_go(token, Command::Finish);
                self.request_over(token, purpose, "PUBLISH_DONE");
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
                self.request_over(token, purpose, "the track is malformed");
            }
            (
                _,
                RequestEvent::TrackNotProcessed { .. } | RequestEvent::TooManyNamespaces { .. },
            ) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, purpose, "the session could not take its answer");
            }
            // LateMessage, the FETCH_OK of a catalog (the catalog comes on its
            // fetch stream), and what is no answer to a request of its kind.
            _ => {}
        }
        Ok(None)
    }

    /// An event of a request stream the relay opened, as the gatherer takes
    /// it.
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
                // Asked for nothing else: it announces no namespace.
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
                    if let PeerRequestEvent::UpdateAccepted { response, .. } = event
                        && fetching.answering.says.try_send(response).is_err()
                    {
                        self.end_fetch(stream, stream_reset_code::EXCESSIVE_LOAD);
                    }
                } else if let Some(request) = self.refused.get(&stream) {
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

    fn reply(&mut self, mut send: Send, response: ControlMessage) {
        self.replies.spawn(async move {
            use xmoq_moqt_io::SendStream;
            let _ = send_message(&mut send, &response, &Trace::default(), "reply").await;
            send.finish();
        });
    }

    fn refuse(&mut self, stream: u64, send: Send, request: PeerRequestId, code: u64) -> Result<()> {
        let response = self
            .session
            .reject(request, code, 0, ReasonPhrase::default())?;
        self.refused.insert(stream, request);
        self.reply(send, response);
        Ok(())
    }

    /// A FETCH of the relay for a range of a transcript, answered from the
    /// pieces kept ([`xmoq_track_serving::fetch`]).
    fn on_fetch(
        &mut self,
        stream: u64,
        send: Send,
        request: PeerRequestId,
        fetch: &Fetch,
    ) -> Result<()> {
        // A Joining FETCH is refused, as the gatherer refuses it: the relay
        // turns a viewer's into a Standalone one before it asks.
        let FetchRange::Standalone {
            full_track_name,
            start,
            end,
        } = &fetch.range
        else {
            return self.refuse(stream, send, request, request_error_code::NOT_SUPPORTED);
        };
        let Some(prefix) = listened_of(full_track_name).and_then(|listened| {
            self.args
                .prefixes
                .iter()
                .position(|prefix| *prefix == listened)
        }) else {
            return self.refuse(stream, send, request, request_error_code::DOES_NOT_EXIST);
        };
        if self
            .fetches
            .values()
            .filter(|fetching| !fetching.over)
            .count()
            >= FETCHES_AT_ONCE
        {
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
        let Some(transcript) = self.transcripts.get(prefix) else {
            return self.refuse(stream, send, request, request_error_code::DOES_NOT_EXIST);
        };
        let (planned, speeches) = match Self::planned(transcript, *start, *end, group_order) {
            Ok(planned) => planned,
            Err(code) => {
                println!("FETCH_REFUSED prefix={prefix} error_code={code:#x}");
                return self.refuse(stream, send, request, code);
            }
        };
        let count = speeches.len();
        let pieces = pieces_of(speeches, planned.left_out);
        let bytes = payload_bytes(&pieces);
        if !self.held.take(bytes) {
            return self.refuse(stream, send, request, request_error_code::EXCESSIVE_LOAD);
        }
        // End Of Track 0: the transcript goes on.
        let (ok, owed) = self
            .session
            .accept_fetch(request, false, planned.end_location, vec![])?;
        let mut messages = vec![ok];
        messages.extend((0..owed).map(|_| ControlMessage::RequestOk(RequestOk::default())));
        println!(
            "FETCH_ANSWERED prefix={prefix} start={},{} end={},{} pieces={count}",
            start.group, start.object, planned.end_location.group, planned.end_location.object,
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

    /// What a FETCH of a transcript gets: the plan, and the pieces in the
    /// order they are sent. The pieces kept are the latest, without a gap:
    /// a range that begins before them gets the End of Unknown Range
    /// indicator.
    fn planned(
        transcript: &Transcript,
        start: Location,
        end: Location,
        group_order: GroupOrder,
    ) -> Result<(Plan, Vec<Kept>), u64> {
        let planned = plan(
            start,
            end,
            transcript.largest(),
            transcript.kept(),
            group_order,
        )?;
        let mut speeches: Vec<Kept> = planned
            .groups
            .map(|(first, last)| {
                transcript
                    .between(first, last)
                    .into_iter()
                    .map(|speech| (speech.number, speech.payload()))
                    .collect()
            })
            .unwrap_or_default();
        if group_order == GroupOrder::Descending {
            speeches.reverse();
        }
        Ok((planned, speeches))
    }

    /// What the task of a FETCH said, as the gatherer takes it (why the
    /// relay's direction is stopped, at its `fetch_over`).
    fn fetch_over(&mut self, stream: u64, answered: Answered) {
        let Some(fetching) = self.fetches.get_mut(&stream) else {
            return;
        };
        match answered {
            Answered::Written => {
                fetching.over = true;
                self.held.give_back(std::mem::take(&mut fetching.bytes));
                self.session.on_fetch_answer_ended(fetching.request);
                if fetching.peer_finished {
                    let request = fetching.request;
                    self.fetches.remove(&stream);
                    self.session.on_peer_request_closed(request);
                }
            }
            Answered::RequestOver { peer_ended: false } | Answered::Stopped | Answered::Failed => {
                self.end_fetch(stream, stream_reset_code::CANCELLED);
            }
            Answered::RequestOver { peer_ended: true } => {}
        }
    }

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

    fn fetch_closed(&mut self, stream: u64, how: PeerRequestClose) {
        let Some(fetching) = self.fetches.get_mut(&stream) else {
            return;
        };
        fetching.peer_finished = true;
        if !how.is_fin() {
            self.end_fetch(stream, stream_reset_code::CANCELLED);
            return;
        }
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
