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
//! the room, and a meeting has a broadcast for each person.
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

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use xmoq_again::next_session::{Pause, new_url};
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::{ObjectStatus, SubgroupObject};
use xmoq_moqt::early::{Arrived, EarlyObjects, held_bytes};
use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, FetchStreamEncoder};
use xmoq_moqt::immutable::{Of, TrackRefusal, searched_integers};
use xmoq_moqt::message::{
    ControlMessage, FetchRange, ReasonPhrase, RequestError, publish_done_status, request_error_code,
};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::{
    ControlEvent, FetchStreamOpened, MalformedRequest, RequestEvent, RequestId, Role, Session,
    SubscribedAlias, Transport,
};
use xmoq_moqt::{RequestFault, stream_reset_code};
use xmoq_moqt_io::wt::WtConnection;
use xmoq_moqt_io::{
    Connection, FetchChunk, PeerRequestStreamEvent, RequestClose, RequestReadError, StreamConfig,
    StreamEnd, StreamStops, UniEvent, client, read_request_streams, read_uni_streams,
};
use xmoq_msf::{CATALOG_TRACK, MAX_CATALOG_LENGTH};
use xmoq_publisher::answers::{self, Bounds, MAX_ANSWERED_AT_ONCE, Note};
use xmoq_publisher::fetch::pieces_of;
use xmoq_publisher::own::OwnRequests;
use xmoq_publisher::publication::{Ending, FIRST_TRACK_ALIAS, Publication};
use xmoq_publisher::requests::{AskedFetch, FetchAnswer, PeerRequests};
use xmoq_publisher::streams::{COMMAND_QUEUE, Command, GroupToSend, Heard, Sending, send_group};
use xmoq_publisher::{shown, sleep_until};
use xmoq_room_token::{Role as TokenRole, RoomKey};
use xmoq_speech::cut::Cut;
use xmoq_speech::judge::{Judge, ModelJudge, VOICE_DBFS};
use xmoq_speech::vad::Silero;
use xmoq_speech::{AudioPacket, Listener};
use xmoq_subscriber::kept::{Becomes, Happened};
use xmoq_subscriber::namespaces::KeptNamespace;
use xmoq_transcriber::args::Args;
use xmoq_transcriber::listen::{Broadcast, BroadcastTrack, CatalogSays, from_of};
use xmoq_transcriber::transcript::Transcript;
use xmoq_transcript_tracks::names::{listened_of, speech_track, transcript_namespace};
use xmoq_transcript_tracks::payloads::Speech;

use crate::reader::{Read, ReaderPool, ToRead};

type Send = <WtConnection as Connection>::Send;

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

/// The bytes of payload the FETCH answers being written hold together
/// (`Held` of `xmoq-publisher`): 16 answers of the default 1,000 pieces
/// of 1 KB. Provisional.
const MAX_BYTES_HELD: usize = 16 * 1000 * 1024;

/// The probability of a voice from which a frame has one: whisper.cpp's
/// default (`whisper_vad_default_params`). Provisional.
const VAD_THRESHOLD: f32 = 0.5;

/// How a session ended without an error.
#[derive(Debug)]
pub enum SessionEnd {
    /// The transcriber was told to stop.
    Stopped,
    /// The relay sent GOAWAY on the control stream (§10.4), with the URL
    /// to go to, or none for the same.
    Moved(Option<String>),
}

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

/// Says that the `SUBSCRIBE_NAMESPACE` for `prefix` is asked for again
/// `wait_ms` from now.
fn watch_ended(prefix: &TrackNamespace, why: &str, wait_ms: u64) {
    println!(
        "WATCH_ENDED prefix={} why={why:?} again_in_ms={wait_ms}",
        shown(prefix)
    );
}

/// Everything of one session.
struct Transcribing<'a> {
    connection: &'a Arc<WtConnection>,
    args: &'a Args,
    reader: &'a mut ReaderPool,
    transcripts: &'a mut [Transcript],
    key: Option<&'a RoomKey>,
    /// The model of voice activity; `None` with `--voice loudness`.
    vad: Option<&'a Path>,
    session: Session,
    stops: StreamStops,
    own: OwnRequests<Purpose>,
    /// The transcript of each prefix.
    published: Vec<Publication<Speech>>,
    next_track_alias: u64,
    watches: Vec<KeptNamespace>,
    broadcasts: HashMap<TrackNamespace, Broadcast<Listener>>,
    /// The tracks of broadcasts to be asked for again, and when.
    asking_again: HashMap<(TrackNamespace, BroadcastTrack), Instant>,
    /// The broadcast each catalog FETCH is for, by its Request ID.
    catalog_fetches: HashMap<u64, TrackNamespace>,
    catalogs: mpsc::Sender<(TrackNamespace, RequestId, Option<Vec<u8>>)>,
    /// Objects read before the `SUBSCRIBE_OK` that gives their Track Alias,
    /// by that alias (`xmoq_moqt::early`).
    early: EarlyObjects<(u64, SubgroupObject)>,
    /// The requests of the relay: the FETCH requests being answered, and
    /// those refused.
    requests: PeerRequests,
    /// The PUBLISH requests that wait for their streams to be closed, by
    /// token, with their prefix.
    ending: HashMap<u64, (usize, Ending)>,
    drained: mpsc::Sender<u64>,
    sends: JoinSet<()>,
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

/// Reads a catalog's fetch stream to its end: the last Object, which is the
/// catalog as it is now.
async fn last_object(
    header: xmoq_moqt::fetch::FetchHeader,
    mut body: xmoq_moqt_io::FetchBody,
) -> Option<Vec<u8>> {
    let mut decoder = FetchStreamDecoder::new(MAX_CATALOG_LENGTH, GroupOrder::Ascending);
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
    kept: &mut client::UntilClosed<WtConnection>,
    args: &Args,
    (reader, transcripts, key, vad): (
        &mut ReaderPool,
        &mut [Transcript],
        Option<&RoomKey>,
        Option<&Path>,
    ),
    read: &mut mpsc::Receiver<Read>,
    stop: &mut watch::Receiver<bool>,
) -> Result<SessionEnd> {
    let stops = StreamStops::default();
    let (uni_queue, mut uni) = mpsc::channel(UNI_QUEUE);
    let stream_config = StreamConfig::default();
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
        own: OwnRequests::new(heard, OPEN_WAIT),
        published: args
            .prefixes
            .iter()
            .map(|_| Publication::new(PIECES_WAITING_FOR_A_PUBLISH))
            .collect(),
        next_track_alias: FIRST_TRACK_ALIAS,
        watches: args
            .prefixes
            .iter()
            .map(|prefix| KeptNamespace::new(prefix.clone(), Pause::SERVING))
            .collect(),
        broadcasts: HashMap::new(),
        asking_again: HashMap::new(),
        catalog_fetches: HashMap::new(),
        catalogs,
        early: EarlyObjects::default(),
        requests: PeerRequests::new(Bounds {
            at_once: MAX_ANSWERED_AT_ONCE,
            bytes: MAX_BYTES_HELD,
            say_queue: COMMAND_QUEUE,
        }),
        ending: HashMap::new(),
        drained,
        sends: JoinSet::new(),
        untimed: 0,
    };

    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    // The relay looks at no namespace or role of the token of a SETUP.
    let token = args.prefixes.first().and_then(|prefix| {
        transcribing.token(&format!("{}/*", namespace_text(prefix)), TokenRole::Sub)
    });
    let options = client::setup_options(token.as_deref(), implementation)?;
    let set_up = client::setup(
        connection.as_ref(),
        &mut transcribing.session,
        &mut uni,
        &mut kept.control,
        options,
        client::SETUP_WAIT,
    )
    .await?;
    println!("SETUP_OK");
    // What came before the SETUP (§3.3) is taken first.
    for event in set_up.early {
        if let Some(ended) = transcribing.on_uni(event)? {
            return Ok(ended);
        }
    }
    for index in 0..transcribing.watches.len() {
        transcribing.watch(index)?;
        transcribing.publish(index)?;
    }

    let ended = loop {
        // WHY taken before the select: its arms borrow parts of the state
        // mutably, the task notes of the FETCH answers among them.
        let (next_watch, next_ask) = (transcribing.next_watch(), transcribing.next_ask());
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break SessionEnd::Stopped;
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
            note = transcribing.requests.next_note() => transcribing.fetch_noted(note),
            Some(token) = drained_publishes.recv() => transcribing.publish_done(token)?,
            () = sleep_until(next_watch) => transcribing.watch_again()?,
            () = sleep_until(next_ask) => transcribing.ask_again()?,
            Some(_) = transcribing.sends.join_next() => {}
        }
    };
    // The pieces under way are read: what was said before the end is not
    // lost to it.
    let namespaces: Vec<TrackNamespace> = transcribing.broadcasts.keys().cloned().collect();
    for namespace in namespaces {
        transcribing.listen_no_more(&namespace);
    }
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
    /// A token of the transcriber's own for `role` in `namespace` from now,
    /// as an AUTHORIZATION TOKEN ([`RoomKey::authorization_token`]); `None`
    /// without `--auth-key-hex`. Made anew for each request, as the
    /// gatherer's (why, at its `token`).
    fn token(&self, namespace: &str, role: TokenRole) -> Option<Vec<u8>> {
        let key = self.key?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        Some(key.authorization_token(namespace, role, now))
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
        watch.asked(Instant::now());
        let prefix = watch.prefix().clone();
        let (request, message) = self.session.subscribe_namespace(prefix, parameters)?;
        self.open(Purpose::Watch(index), request, message, &[]);
        Ok(())
    }

    fn next_watch(&self) -> Option<Instant> {
        self.watches
            .iter()
            .filter_map(KeptNamespace::again_at)
            .min()
    }

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

    /// Opens the stream of a request.
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

    /// Tells the task of a request stream something; where its queue is
    /// full the request is cancelled, as the gatherer does (why, at its
    /// `tell`).
    fn tell(&mut self, token: u64, command: Command) {
        let full = self.own.tell(token, command).is_err();
        if full
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

    // FIXME: take the steps of a PUBLISH (`publish` to `publish_done`,
    // `send_live`) from `xmoq-publisher`; the gatherer has a copy of each.
    /// Publishes the transcript of a prefix, unless it is published or waits
    /// to be published again.
    fn publish(&mut self, prefix: usize) -> Result<()> {
        let Some(listened) = self.args.prefixes.get(prefix).cloned() else {
            return Ok(());
        };
        let Some(entry) = self.published.get(prefix) else {
            return Ok(());
        };
        if !entry.to_publish(Instant::now()) {
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
            entry.published(token, track_alias);
        }
        Ok(())
    }

    /// The PUBLISH of a transcript is over; it is published again after a
    /// pause, on the next piece.
    fn unpublish(&mut self, prefix: usize, token: u64) {
        if let Some(entry) = self.published.get_mut(prefix)
            && entry.unpublished(token, true, Instant::now())
        {
            println!("TRANSCRIPT_UNPUBLISHED prefix={prefix}");
        }
    }

    /// Ends the PUBLISH of a transcript with `status` once its streams being
    /// sent are closed ([`Self::publish_done`]), as the gatherer ends a
    /// room's.
    fn end_publish(&mut self, prefix: usize, status: u64) {
        let Some(ending) = self
            .published
            .get_mut(prefix)
            .and_then(|entry| entry.end(status))
        else {
            return;
        };
        let drained = self.drained.clone();
        let when_drained = ending.when_drained();
        tokio::spawn(async move {
            let _ = drained.send(when_drained.await).await;
        });
        self.ending.insert(ending.token, (prefix, ending));
    }

    /// Every stream of an ending PUBLISH is closed: its `PUBLISH_DONE` with
    /// the number of streams (§10.11), and the transcript is published anew.
    fn publish_done(&mut self, token: u64) -> Result<()> {
        let Some((prefix, ending)) = self.ending.remove(&token) else {
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
                "TRANSCRIPT_DONE prefix={prefix} status={:#x} streams={streams}",
                ending.status
            );
        }
        self.publish(prefix)
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
        let is_live = self.published.get(prefix).is_some_and(Publication::live);
        if !is_live {
            self.publish(prefix)?;
            if let Some(entry) = self.published.get_mut(prefix) {
                entry.wait(speech);
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
            track_alias: entry.track_alias(),
            group_id: speech.number,
            payload: speech.payload(),
            opened: Arc::clone(entry.opened()),
        };
        let counted = Sending::begin(entry.sending());
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
        let Some(from) = from_of(namespace) else {
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
            Broadcast::new(
                prefix,
                from,
                Listener::new(self.judge()?, Cut::default())?,
                self.args.ask_again_times,
            ),
        );
        self.subscribe_catalog(namespace)
    }

    /// Subscribes to the catalog of a broadcast.
    fn subscribe_catalog(&mut self, namespace: &TrackNamespace) -> Result<()> {
        self.asking_again
            .remove(&(namespace.clone(), BroadcastTrack::Catalog));
        let Some(broadcast) = self.broadcasts.get(namespace) else {
            return Ok(());
        };
        if broadcast.catalog.is_some() {
            return Ok(());
        }
        let parameters = self.subscribe_parameters(broadcast.prefix);
        let track = FullTrackName::new(namespace.clone(), CATALOG_TRACK.as_bytes().to_vec())?;
        let Some((request, message)) = self.subscribe(namespace, track, &parameters) else {
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

    /// A SUBSCRIBE for a track of a broadcast, or `None` where the session
    /// takes no more requests (§9.4 after a GOAWAY, or with its Request IDs
    /// used up): the broadcast is listened to no more, so that it does not
    /// hold a place of `--tracks` with nothing asked.
    fn subscribe(
        &mut self,
        namespace: &TrackNamespace,
        track: FullTrackName,
        parameters: &[Parameter],
    ) -> Option<(RequestId, ControlMessage)> {
        match self.session.subscribe(track, parameters.to_vec()) {
            Ok(subscribed) => Some(subscribed),
            Err(error) => {
                println!(
                    "BROADCAST_NOT_ASKED namespace={} why={error}",
                    shown(namespace)
                );
                self.listen_no_more(namespace);
                None
            }
        }
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
        let Some(broadcast) = self.broadcasts.get_mut(namespace) else {
            return Ok(());
        };
        match broadcast.on_catalog(payload) {
            CatalogSays::Unread(error) => {
                println!(
                    "CATALOG_UNREAD namespace={} error={error}",
                    shown(namespace)
                );
                Ok(())
            }
            CatalogSays::WithoutAudio => {
                println!("CATALOG_WITHOUT_AUDIO namespace={}", shown(namespace));
                Ok(())
            }
            CatalogSays::Same => Ok(()),
            CatalogSays::Listen { instead_of } => {
                if let Some(token) = instead_of {
                    self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                    self.end_piece(namespace);
                }
                self.subscribe_audio(namespace)
            }
        }
    }

    /// Subscribes to the audio track of a broadcast.
    fn subscribe_audio(&mut self, namespace: &TrackNamespace) -> Result<()> {
        self.asking_again
            .remove(&(namespace.clone(), BroadcastTrack::Audio));
        let Some(broadcast) = self.broadcasts.get(namespace) else {
            return Ok(());
        };
        let (Some(name), None) = (broadcast.audio_name().map(<[u8]>::to_vec), broadcast.audio)
        else {
            return Ok(());
        };
        let parameters = self.subscribe_parameters(broadcast.prefix);
        let track = FullTrackName::new(namespace.clone(), name)?;
        let Some((request, message)) = self.subscribe(namespace, track, &parameters) else {
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

    /// A request of the transcriber was refused, and was let go of.
    fn refused(&mut self, token: u64, purpose: &Purpose, error: &RequestError) {
        let why = format!("refused with {:#x}", error.error_code);
        let over = Happened::Refused {
            error_code: error.error_code,
            named_ms: error.retry_wait_ms(),
        };
        match purpose {
            Purpose::Watch(index) => {
                if let Some(watch) = self.watches.get_mut(*index) {
                    let wait = watch.refused(error.retry_wait_ms(), Instant::now());
                    watch_ended(watch.prefix(), &why, wait);
                }
            }
            Purpose::Audio(namespace) => {
                self.track_over(&namespace.clone(), BroadcastTrack::Audio, over, &why);
            }
            Purpose::Catalog(namespace) => {
                self.track_over(&namespace.clone(), BroadcastTrack::Catalog, over, &why);
            }
            _ => self.request_over(token, purpose, Happened::Ended, &why),
        }
    }

    /// A track of a broadcast that stays announced is over: the piece under
    /// way is read where it is the audio track, and the track is asked for
    /// again after a pause by the rule every client asks by, or the
    /// broadcast listened to no more until its NAMESPACE comes again
    /// (`listen::keeping`).
    fn track_over(
        &mut self,
        namespace: &TrackNamespace,
        track: BroadcastTrack,
        happened: Happened,
        why: &str,
    ) {
        let word = track.word().to_uppercase();
        println!(
            "{word}_NOT_SUBSCRIBED namespace={} why={why:?}",
            shown(namespace)
        );
        if track == BroadcastTrack::Audio {
            self.end_piece(namespace);
        }
        let Some(broadcast) = self.broadcasts.get_mut(namespace) else {
            return;
        };
        match broadcast.on(track, happened) {
            Becomes::Again(pause) => self.ask_later(namespace, track, pause),
            Becomes::GivenUp | Becomes::Subscribed => {
                println!(
                    "NOT_ASKING_AGAIN namespace={} track={}",
                    shown(namespace),
                    track.word()
                );
                self.listen_no_more(namespace);
            }
        }
    }

    /// A track of a broadcast is asked for again in `pause_ms`.
    fn ask_later(&mut self, namespace: &TrackNamespace, track: BroadcastTrack, pause_ms: u64) {
        println!(
            "ASKING_AGAIN namespace={} track={} in_ms={pause_ms}",
            shown(namespace),
            track.word()
        );
        self.asking_again.insert(
            (namespace.clone(), track),
            Instant::now() + Duration::from_millis(pause_ms),
        );
    }

    fn next_ask(&self) -> Option<Instant> {
        self.asking_again.values().min().copied()
    }

    fn ask_again(&mut self) -> Result<()> {
        let now = Instant::now();
        let due: Vec<(TrackNamespace, BroadcastTrack)> = self
            .asking_again
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(asked, _)| asked.clone())
            .collect();
        for (namespace, track) in due {
            match track {
                BroadcastTrack::Catalog => self.subscribe_catalog(&namespace)?,
                BroadcastTrack::Audio => self.subscribe_audio(&namespace)?,
            }
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
        self.asking_again.retain(|(asked, _), _| asked != namespace);
        let Some(broadcast) = self.broadcasts.remove(namespace) else {
            return;
        };
        for token in broadcast.tokens() {
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
        if track.name() == CATALOG_TRACK.as_bytes() {
            return self.on_catalog(&namespace.clone(), &object.payload);
        }
        if broadcast.audio_name() == Some(track.name()) {
            self.on_audio(&namespace.clone(), object);
        }
        Ok(())
    }

    /// An event of the relay's unidirectional streams.
    fn on_uni(&mut self, event: UniEvent) -> Result<Option<SessionEnd>> {
        // What a SUBSCRIBE that ended meanwhile left held is let go of before
        // what came after it.
        self.take_early()?;
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
            UniEvent::ReaderPanicked => return Err(client::ReaderPanicked.into()),
        }
        Ok(None)
    }

    /// What the task of one of the transcriber's request streams heard.
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
                self.request_over(
                    token,
                    &purpose,
                    Happened::Ended,
                    &format!("its stream ended ({how})"),
                );
                Ok(None)
            }
            Heard::Stopped(code) => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(
                    token,
                    &purpose,
                    Happened::Ended,
                    &format!("stopped ({code:?})"),
                );
                Ok(None)
            }
            Heard::Failed(RequestReadError::Peer(error)) => Err(error.into()),
            Heard::Failed(RequestReadError::Lost) => bail!("the connection is gone"),
        }
    }

    /// A request of the transcriber is over, and was let go of.
    /// The track of `request` is malformed, and the session ended the
    /// request: §2.4.2, "it MUST cancel any corresponding subscription or
    /// fetches for that Track". Its stream is reset with `MALFORMED_TRACK`.
    fn malformed(&mut self, request: RequestId) {
        const WHY: &str = "the track is malformed";
        self.catalog_fetches.remove(&request.value());
        let token = self.own.token_of(request);
        let Some(token) = token else {
            return;
        };
        let Some(purpose) = self.let_go(token, Command::Cancel(stream_reset_code::MALFORMED_TRACK))
        else {
            return;
        };
        match &purpose {
            Purpose::Catalog(namespace) => {
                self.track_over(
                    &namespace.clone(),
                    BroadcastTrack::Catalog,
                    Happened::Malformed,
                    WHY,
                );
            }
            Purpose::Audio(namespace) => {
                self.track_over(
                    &namespace.clone(),
                    BroadcastTrack::Audio,
                    Happened::Malformed,
                    WHY,
                );
            }
            purpose => self.request_over(token, purpose, Happened::Ended, WHY),
        }
    }

    /// The relay sent GOAWAY on the stream of a request of the transcriber:
    /// the request alone is asked again, on this session, as after its end
    /// (§10.4: "the endpoint SHOULD re-issue that specific request ... and
    /// close the old request stream using the appropriate mechanism (e.g.
    /// FIN, stream reset, or `PUBLISH_DONE`)"). A subscription and the FETCH
    /// for the catalog are cancelled (§5.1, §5.2), and the PUBLISH of a
    /// transcript ends with `PUBLISH_DONE` `GOING_AWAY` once its streams are
    /// closed (§10.11) and is sent again. The FETCH is not asked again, as
    /// after any other end of it: the catalog's subscription goes on.
    ///
    /// FIXME: a New Session URI names another session for the request, and
    /// the request is asked again on this one (§10.4: "on a session at the
    /// specified URI"); the transcriber keeps one session.
    fn request_moved(&mut self, token: u64, purpose: &Purpose) {
        match purpose {
            Purpose::Speech(prefix) => {
                if self
                    .published
                    .get(*prefix)
                    .is_some_and(|entry| entry.publish() == Some(token))
                {
                    self.end_publish(*prefix, publish_done_status::GOING_AWAY);
                }
            }
            purpose => {
                self.let_go(token, Command::Cancel(stream_reset_code::CANCELLED));
                self.request_over(token, purpose, Happened::Ended, "GOAWAY");
            }
        }
    }

    fn request_over(&mut self, token: u64, purpose: &Purpose, end: Happened, why: &str) {
        match purpose {
            Purpose::Watch(index) => {
                if let Some(watch) = self.watches.get_mut(*index) {
                    let wait = watch.ended(Instant::now());
                    watch_ended(watch.prefix(), why, wait);
                }
            }
            Purpose::Catalog(namespace) => {
                self.track_over(&namespace.clone(), BroadcastTrack::Catalog, end, why);
            }
            Purpose::CatalogFetch(namespace) => {
                if let Some(broadcast) = self.broadcasts.get_mut(namespace) {
                    broadcast.catalog_fetch = None;
                }
            }
            Purpose::Audio(namespace) => {
                self.track_over(&namespace.clone(), BroadcastTrack::Audio, end, why);
            }
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
        self.stops.limit_payload(track_alias, MAX_CATALOG_LENGTH);
        let Some(catalog) = self.broadcasts.get_mut(namespace).and_then(|broadcast| {
            broadcast.on(BroadcastTrack::Catalog, Happened::Subscribed);
            broadcast.catalog
        }) else {
            return Ok(());
        };
        let Some(own) = self.own.get(catalog) else {
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
    /// (`xmoq_publisher::answers::PEER_END_WAIT`). WHY: a request left
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
        let waiting = self
            .published
            .get_mut(prefix)
            .map(|entry| entry.established(token, forward))
            .unwrap_or_default();
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
                    broadcast.on(BroadcastTrack::Audio, Happened::Subscribed);
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
                    .is_some_and(|entry| entry.publish() == Some(token))
                {
                    self.end_publish(prefix, publish_done_status::UPDATE_FAILED);
                }
            }
            (_, RequestEvent::Rejected(error)) => {
                self.let_go(token, Command::Finish);
                self.refused(token, purpose, &error);
            }
            (Purpose::Audio(namespace), RequestEvent::SubscriptionDone(done)) => {
                let namespace = namespace.clone();
                self.let_go(token, Command::Finish);
                let why = format!("PUBLISH_DONE {:#x}", done.status_code);
                self.track_over(&namespace, BroadcastTrack::Audio, Happened::Ended, &why);
            }
            (_, RequestEvent::SubscriptionDone(_)) => {
                self.let_go(token, Command::Finish);
                self.request_over(token, purpose, Happened::Ended, "PUBLISH_DONE");
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
                self.request_over(
                    token,
                    purpose,
                    Happened::Ended,
                    "the session could not take its answer",
                );
            }
            // LateMessage, the FETCH_OK of a catalog (the catalog comes on its
            // fetch stream), and what is no answer to a request of its kind.
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

    /// A FETCH of the relay: for a range of a transcript, which is answered
    /// from the pieces kept ([`xmoq_publisher::fetch`]).
    fn on_fetch(&mut self, asked: AskedFetch<Send>) -> Result<()> {
        // A Joining FETCH is refused, as the gatherer refuses it: the relay
        // turns a viewer's into a Standalone one before it asks.
        let FetchRange::Standalone {
            full_track_name,
            start,
            end,
        } = asked.fetch.range.clone()
        else {
            return self.refuse(asked, request_error_code::NOT_SUPPORTED);
        };
        let Some(prefix) = listened_of(&full_track_name).and_then(|listened| {
            self.args
                .prefixes
                .iter()
                .position(|prefix| *prefix == listened)
        }) else {
            return self.refuse(asked, request_error_code::DOES_NOT_EXIST);
        };
        if self.requests.full() {
            return self.refuse(asked, request_error_code::EXCESSIVE_LOAD);
        }
        let group_order = answers::group_order(&asked.fetch);
        let Some(transcript) = self.transcripts.get(prefix) else {
            return self.refuse(asked, request_error_code::DOES_NOT_EXIST);
        };
        let (planned, speeches) = match transcript.fetch(start, end, group_order) {
            Ok(planned) => planned,
            Err(code) => {
                println!("FETCH_REFUSED prefix={prefix} error_code={code:#x}");
                return self.refuse(asked, code);
            }
        };
        let count = speeches.len();
        let pieces = pieces_of(speeches, planned.left_out);
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
            "FETCH_ANSWERED prefix={prefix} start={},{} end={},{} pieces={count}",
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
