//! Connects the sessions of the relay to its one decision maker.
//!
//! Every session has readers that turn its streams into [`Event`]s. One task,
//! the hub, feeds them to the [`Relay`] in the order they arrive and carries out
//! the [`Action`]s it returns. The hub never waits for a peer and never writes
//! to anything that can block: what it hands to a session goes into a bounded
//! queue, and a queue that is full ends that session or that subscription
//! instead of holding up the others.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use xmoq_moqt::data::SubgroupObject;
use xmoq_moqt::fetch::{FetchHeader, FetchObject, FetchStreamEncoder};
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt::params::GroupOrder;
use xmoq_moqt::printable::{printable, printable_namespace};
use xmoq_moqt::session::MAX_UNANSWERED_UPDATES;
use xmoq_moqt::{TerminationCode, stream_reset_code};
use xmoq_moqt_io::{
    Connection, ConnectionEnd, FetchBody, FetchChunk, FetchStreamWriter, IncomingStreams,
    OutgoingEvent, OutgoingSubgroup, PeerRequestClose, PeerRequestStreamEvent, QueueFull,
    RecvStream, RequestClose, RequestReadError, RequestReader, RequestStreamEvent, SendBudget,
    SendLimits, SendStream, StreamConfig, StreamEnd, StreamStops, Trace, UniEvent, WriteError,
    open_fetch_stream_in_order, open_fetch_stream_without_header, read_request_streams,
    read_uni_streams, send_message,
};
use xmoq_relay_core::{
    Action, Event, FetchPiece, FetchPipeEnd, GoingAway, LimitsOfASession, OutStreamKey,
    OwnRequestEnd, OwnRequestKey, Relay, RequestEnd, Role, SendLimit, SessionKey, StreamKey,
};

/// Limits of the relay. Every value is provisional: the draft sets none of
/// them, and `XmoQ` has not decided any.
///
/// What one session can make the relay hold, with the defaults here and the
/// QUIC limits the relay is started with (`quic_limits` in `main.rs`), and
/// what sets each:
///
/// | Held | Bound | Set by |
/// |---|---|---|
/// | QUIC: received on its streams and not read yet | 9.375 MiB (`receive_window`) | `quic_limits` |
/// | QUIC: written to its streams and not acknowledged | 4 MiB (`send_window`) | `quic_limits` |
/// | QUIC: datagrams it sent, which nothing reads | 1500 bytes | the transport |
/// | Read buffers of its streams | 64 + 264 streams of 16 KiB: about 5 MiB | `streams` |
/// | Undecoded bytes of its subgroup streams | 4 MiB (`session_buffer_limit`) | `streams` |
/// | Streams held for a Track Alias that is not known | 1 MiB, counted with `held_bytes` (`MOST_HELD_BYTES`) | `xmoq-moqt`'s `early` |
/// | Objects waiting to be written to it | 8 MiB (`send.bytes`); 4 MiB of one subscription (`send_for_one_subscription.bytes`) | `--session-send-bytes`, `--subscription-send-bytes` |
/// | Messages waiting to be written to it | 256 a stream (`session_queue_len`) | here |
///
/// At worst, with the defaults, a session holds the sum, about 31 MiB, and
/// the relay
///
/// `max_sessions × 23.4 MiB + min(max_sessions × send.bytes, send_for_the_relay_bytes)`
///
/// (23.4 MiB being the rows but the Objects that wait), about 25 GiB for
/// 1,000 sessions, of which 2 GiB are Objects that wait, plus the messages
/// and what the events that wait for the hub hold (shared, 1024 events).
/// These are bounds for the flags to fit to the machine (decided on
/// 2026-10-25: the numbers of viewers are goals of the design, and what a
/// machine holds is set when it is started); what was measured is far
/// less, 192 MB for an edge of 500 viewers. Without the QUIC limits the
/// first two rows are those of the transport library, which for quinn's
/// defaults are far larger than everything else here.
#[derive(Clone)]
pub struct HubConfig {
    /// Limits and deadlines of the streams of each session. Its
    /// `max_request_streams` is not read: [`Self::stream_config`] has it
    /// follow [`Self::limits_of_a_session`].
    pub streams: StreamConfig,
    /// Sessions of clients served at once, the one with the parent relay
    /// not counted. One more is sent to [`Self::when_full`] where there is
    /// one, and closed at once otherwise. **Provisional:
    /// 1,000** (decided on 2026-10-25 that a full relay sends the next
    /// session to another; the number is the 500 viewers a relay is to
    /// serve, at two sessions a page).
    pub max_sessions: usize,
    /// Where a session goes that comes when [`Self::max_sessions`] are
    /// served: the URL a GOAWAY right after the relay's SETUP names (§3.6,
    /// §10.4), with [`SENT_AWAY_QUERY`] added. `None`: it is closed.
    ///
    /// WHY a GOAWAY after SETUP: it is the one thing of the draft that names
    /// another place for a whole session, and the clients here follow it.
    /// WHY NOT refuse before SETUP: nothing before SETUP carries a URI, and
    /// a client refused by the WebTransport handshake knows only to come
    /// back. WHY NOT REDIRECT (§10.6.1) on each request: the session would
    /// keep its place, and no client here follows a REDIRECT.
    ///
    /// No certificate hash goes with it: a GOAWAY carries a URI alone, and a
    /// client that pinned a hash pins the same one at the URI (the pages of
    /// `interop/page.ts`, `new_url` of `xmoq-viewer-io`), so the relay there
    /// has to have the same certificate, or one under a CA the clients take.
    pub when_full: Option<String>,
    /// Events from all sessions that wait for the hub. Readers wait beyond
    /// that, which holds their peers back through flow control.
    pub event_queue_len: usize,
    /// Control messages that wait to be written to one session, and messages
    /// that wait on one of its request streams. A session that lets them pile
    /// up further is closed.
    pub session_queue_len: usize,
    /// What may wait to be written to one subscriber's session, over all its
    /// subgroup streams. A session that lets more pile up loses the
    /// subscription whose streams carry the most (`TOO_FAR_BEHIND`).
    /// **Provisional: 8 MiB, 16,384 Objects, 4,096 streams**
    /// (`--session-send-bytes`, `--session-send-objects`,
    /// `--session-send-streams`). Its `delivery_patience` is that of every
    /// subgroup stream of the relay (`--delivery-patience-ms`).
    ///
    /// WHY end the subscription of a subscriber that falls behind, which can
    /// subscribe again and resume at the next key frame: WHY NOT make the
    /// publisher wait, as one viewer that does not read would stall the
    /// broadcast for everyone. WHY NOT drop the queued Objects one by one: a
    /// missing delta frame makes H.264 and VP8 decoders show corrupt
    /// pictures without an error
    /// (docs/experiments/2026-10-10-webcodecs-loopback.md).
    ///
    /// WHY 4,096 streams: a participant of a meeting of 49 receives 48
    /// voices of an Object a stream, 2,400 streams a second, and a stream
    /// counts until the receiver's transport acknowledged it, so that the
    /// 256 of before were reached at a round trip of 107 ms with nothing
    /// late; with 4,096 the round trip and the delay together may be 1.7
    /// seconds. 16,384 Objects are 6.3 seconds of those voices and 8 videos
    /// of 25 frames a second; 8 MiB are 9.3 seconds of them at 67 kbit/s a
    /// voice and 500 kbit/s a video. An edge counts here as one session:
    /// its 500 viewers of one broadcast are one subscription a track at the
    /// relay above, which the limits of a subscription end first.
    pub send: SendLimits,
    /// What may wait to be written to one subscription, over all its
    /// subgroup streams. A subscription that lets more pile up ends
    /// (`TOO_FAR_BEHIND`), and the others of its session go on.
    /// **Provisional: 4 MiB, 4,096 Objects, 1,024 streams**
    /// (`--subscription-send-bytes`, `--subscription-send-objects`,
    /// `--subscription-send-streams`). Its `delivery_patience` is not read:
    /// that of [`Self::send`] holds.
    ///
    /// WHY a limit of each subscription beside the session's (decided on
    /// 2026-10-25): with the session's alone, one track that falls behind
    /// uses up what the others of the session have, a video the sound's of
    /// a viewer, and an edge's one track every track of its viewers. WHY
    /// 1,024 streams: a sound of 50 Objects a second, an Object a stream,
    /// holds them for 20.5 seconds, and a video of 2.65 Mbit/s fills its 4
    /// MiB in 12.7: behind a narrow link the video ends first, and the
    /// sound goes on, as a viewer lost its sound otherwise
    /// (docs/experiments/2026-10-14-three-minutes-behind-a-narrow-link-and-the-relays-limit-for-one-viewer.md).
    pub send_for_one_subscription: SendLimits,
    /// The payload bytes that may wait to be written to all the
    /// subscribers of the relay together. Where they would be more, the
    /// subscription of the relay whose streams carry the most ends
    /// (`TOO_FAR_BEHIND`), of whichever session; the Object that took the
    /// relay over is queued all the same (`keep_to_the_relay_s_bound`).
    /// **Provisional: 2 GiB** (`--relay-send-bytes`; decided on 2026-10-25).
    ///
    /// WHY a bound of the relay's own: the bound of a session times
    /// [`Self::max_sessions`] is 8 GiB, which is what a deployment does not
    /// size a machine for, and the sessions do not all fall behind at once
    /// in what was measured: one edge of 500 viewers took 192 MB in all
    /// (docs/experiments/2026-10-25-one-video-track-through-two-edges.md).
    /// WHY the one that carries the most, of any session: it is the one
    /// whose end frees the most, and the one that most likely holds the
    /// rest up; ending the one that went over would end a small track
    /// that comes often, a sound, for the sake of a large one elsewhere.
    /// WHY bytes alone: Objects and streams are bounded by the sessions'.
    pub send_for_the_relay_bytes: usize,
    /// How long the last Objects of a track are waited for after
    /// `PUBLISH_DONE` (§10.11 of `draft-ietf-moq-transport-18`).
    pub drain_timeout: Duration,
    /// How long a subgroup stream towards a subscriber may take to open,
    /// from when the relay was told to open it, before the log has a line
    /// for it (`DATA_OPENED`); 0 for a line for every stream. WHY not
    /// every stream of itself: a track of audio opens one every 20
    /// milliseconds for every subscriber. On loopback a stream opens in
    /// well under a millisecond.
    pub slow_open: Duration,
    /// The key of the experiment in authorization (`crate::auth`). None:
    /// every request is allowed.
    pub auth_key: Option<Vec<u8>>,
    /// With a key: a client's SETUP has to carry a token as well. On unless
    /// it is taken out; without a key it means nothing.
    pub auth_setup: bool,
    /// How long a stream is held for a Track Alias that is not known yet
    /// (§11.4.2: "buffer it for a brief period").
    pub early_stream_timeout: Duration,
    /// How long a peer gets to close its direction of a request stream after
    /// the relay finished its own with its last word on the request: a
    /// refusal, `PUBLISH_DONE` or the end of a FETCH
    /// ([`Action::FinishRequest`]). Then the relay stops the stream.
    /// **Provisional: 2 seconds**, also for a request the session did not
    /// keep. What was read before the stop took effect is dropped, and the
    /// session stays open.
    ///
    /// WHY a limit: the draft does not make the subscriber close the stream,
    /// and until it does the request counts against the limit of open ones
    /// (`MAX_PEER_REQUESTS` of `xmoq-moqt`): a peer that sends requests which
    /// are refused and ends none of their streams fills it with them, after
    /// which every request of that session gets `EXCESSIVE_LOAD`. WHY 2
    /// seconds: it is the time a subscriber has after `PUBLISH_DONE`
    /// ([`Self::drain_timeout`]).
    ///
    /// The wait is one of the [`Self::max_timers_per_session`] timers; a
    /// session that has that many gets the direction stopped at once. A
    /// request stream the relay resets is not timed: the request is
    /// forgotten then or at once (`end_answered_fetch` of `xmoq-relay-core`).
    pub request_close_timeout: Duration,
    /// How long the relay goes on reading what had arrived on a connection
    /// after the connection is gone, before it treats the session as closed.
    pub closed_read_timeout: Duration,
    /// Timers one session may have running at once. A timer beyond that is
    /// not started and its event is reported at once.
    ///
    /// What that does is at `Hub::set_timer`: the subscribers of an
    /// unanswered upstream SUBSCRIBE get `TIMEOUT`, a held stream is
    /// stopped, a draining track ends, a peer's direction of a request
    /// stream the relay finished is stopped. The timer for that last one,
    /// and the one for the wait before the relay finishes the request stream
    /// of a FETCH it served, go when the peer ended its direction, so a peer
    /// that ends its streams holds none.
    pub max_timers_per_session: usize,
    /// `xmoq_relay_core::ANNOUNCEMENT_EVERY_MS`, where it is not the relay's
    /// own number: the time in which a session may send one
    /// `PUBLISH_NAMESPACE`.
    pub announcement_every_ms: Option<u64>,
    /// `xmoq_relay_core::MAX_ANNOUNCEMENTS_AT_ONCE`, where it is not the
    /// relay's own number.
    pub announcements_at_once: Option<u32>,
    /// For measuring: every so often, a line for each session of what its
    /// transport and its subgroup streams have (`STATS`). None: no line.
    pub stats_every: Option<Duration>,
    /// For measuring: where an `ARRIVED_STREAM` line for every subgroup
    /// stream of a peer goes, and an `ARRIVED` line for every Object on one,
    /// with the time the hub took it. The other end is `--log-written` of
    /// the Rust client's publisher. None: no line.
    pub log_arrivals: Option<Log>,
    /// How long a subscriber has for the whole of a fetch stream the relay
    /// writes from its cache, at the least. Provisional.
    pub cached_fetch_whole_ms: u64,
    /// The rate, in kbit/s, a subscriber has to take such a stream at, at
    /// the least: the stream has as long as its Objects take at that rate
    /// where that is longer than [`Self::cached_fetch_whole_ms`].
    /// Provisional.
    pub cached_fetch_least_kbps: u64,
    /// `xmoq_relay_core::MAX_BYTES_HELD_BEYOND_THE_CACHE`, where it is not
    /// the relay's own number: how many bytes the fetch streams the relay
    /// writes may keep alive of Groups its caches have let go of.
    pub held_beyond_cache_bytes: Option<usize>,
    /// What one session may have at the relay
    /// ([`xmoq_relay_core::LimitsOfASession`]). The hub reads as many
    /// request streams at once as `peer_requests` asks
    /// ([`request_streams_for`]), whatever
    /// [`StreamConfig::max_request_streams`] of [`Self::streams`] says.
    pub limits_of_a_session: LimitsOfASession,
    /// `xmoq_relay_core::MAX_OWN_REQUESTS_FOR_ONE_SUBSCRIBER`, where it is
    /// not the relay's own number: how many requests the relay has open on
    /// one publisher for one subscriber's session.
    pub requests_for_one_subscriber: Option<usize>,
    /// `xmoq_relay_core::MAX_OWN_FETCHES_FOR_ONE_SUBSCRIBER`, in the same
    /// way: how many FETCH requests the relay has open on one publisher for
    /// one subscriber's session. Counted apart from the SUBSCRIBEs above.
    pub fetches_for_one_subscriber: Option<usize>,
    /// How long a publisher gets to answer a SUBSCRIBE the relay sent it for
    /// a track of a namespace it announced. Then the subscribers that wait
    /// are answered with `TIMEOUT` (§10.6.2).
    pub upstream_timeout: Duration,
    /// How long a client has for its SETUP, from when its session was
    /// taken. Then the session is closed with `CONTROL_MESSAGE_TIMEOUT`.
    /// **Provisional**: the time the clients give a server's SETUP.
    ///
    /// WHY a limit: anybody can connect, and a session that says nothing
    /// holds its place among the sessions for as long as its connection
    /// lives, which the keep-alives of the two ends see to.
    pub setup_timeout: Duration,
    /// How long a session that has sent its SETUP may hold nothing: no
    /// request of its own that is open or held, and none the relay has on
    /// it. Then it is closed with `NO_ERROR`. **Provisional.**
    ///
    /// WHY a limit: the relay serves [`Self::max_sessions`] sessions, and
    /// one that asks for nothing holds its place for as long as its
    /// connection lives, which the keep-alives of the two ends see to; as
    /// many such sessions as there are places keep everybody else out.
    /// WHY longer than [`Self::setup_timeout`]: a client that has sent its
    /// SETUP may be one that is about to announce, or a viewer between two
    /// broadcasts. §13.6.1 has a client "Accept that idle connections can
    /// close and implement reconnection logic when needed".
    ///
    /// The own clients do not come to it in what they do: a publisher
    /// announces as soon as it is connected and holds that; a viewer
    /// subscribes at once, and where its SUBSCRIBE is refused or its
    /// subscription ends it closes its session and connects again or leaves;
    /// the demo's pages and the terminal viewer with a room connect when the
    /// room has a broadcast.
    pub idle_session_timeout: Duration,
    /// Sent as `MOQT_IMPLEMENTATION`.
    pub implementation: String,
    /// Where one line about each thing the relay does goes, if anywhere.
    pub log: Option<Log>,
}

impl HubConfig {
    /// How long a subscriber has for the whole of a fetch stream the relay
    /// writes from its cache, whose Objects count for `bytes`
    /// (`object_size` of `xmoq-relay-core`, their payload and properties):
    /// [`Self::cached_fetch_whole_ms`], or as long as the bytes take at
    /// [`Self::cached_fetch_least_kbps`] where that is longer.
    ///
    /// WHY by the bytes as well: a Group may be 8 MiB, which a viewer on a
    /// link of 2 Mbit/s took 35 seconds for (measured 2026-10-13), and one
    /// that is cut then has no way to get it. At 500 kbit/s such a Group
    /// has 134 seconds, four times that. WHY a least rate, and not only a
    /// time: the time a slow reader may keep a Group alive is then in
    /// proportion to what it is given.
    ///
    /// What it does not reach: what fits into the windows of the connection
    /// is written at once and waits there, so a small Group is never slow; a
    /// fetch stream that is passed on from a publisher; and the Groups
    /// several such streams keep alive together (known gap on a sender with
    /// several sessions in `docs/moqt-implementation-notes.md`).
    fn cached_fetch_whole(&self, bytes: usize) -> Duration {
        let bits = u64::try_from(bytes).unwrap_or(u64::MAX).saturating_mul(8);
        // kbit/s is bits a millisecond.
        let at_the_least_rate = bits / self.cached_fetch_least_kbps.max(1);
        Duration::from_millis(self.cached_fetch_whole_ms.max(at_the_least_rate))
    }
}

/// What a full relay adds to the query of the URL it sends a session to
/// ([`HubConfig::when_full`]). A session that came with it is closed where
/// the relay is full, and not sent on.
///
/// WHY: two relays that each name the other are the natural pair, and a
/// client cannot stop the round by itself (the watching page and the
/// terminal viewer go at once after one GOAWAY, and after a pause after the
/// next, without end). WHY NOT more than one step: where the second relay
/// is full as well, the place to find a relay with room is the one that
/// hands out relays, which is not here yet.
pub const SENT_AWAY_QUERY: &str = "sent-away=1";

/// The Timeout of the GOAWAY a session that is sent away gets, and how long
/// it has to go before it is closed with `GOAWAY_TIMEOUT`. **Provisional.**
/// WHY short: the session holds a place beyond [`HubConfig::max_sessions`]
/// and has nothing to carry; the clients here go at once.
pub const SEND_AWAY_TIMEOUT_MS: u64 = 2_000;

/// How many sessions may be being sent away at once, beyond
/// [`HubConfig::max_sessions`]. One more is closed at once. **Provisional.**
/// WHY a bound: each is a connection the relay holds, as long as
/// [`SEND_AWAY_TIMEOUT_MS`]. WHY 64: as many as the handshakes the relay
/// takes at once (`MAX_HANDSHAKES` of `main.rs`).
pub const MAX_SENDING_AWAY: usize = 64;

/// `GOAWAY_TIMEOUT` (§3.5): "The session was closed because the peer took
/// too long to close the session in response to a GOAWAY".
pub const GOAWAY_TIMEOUT: u32 = 0x10;

/// How a session came to the relay ([`HubHandle::add_session`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// As any session does.
    First,
    /// Sent by a relay that was full: its URL has [`SENT_AWAY_QUERY`].
    SentAway,
}

impl Arrival {
    /// How the session of a URL whose query is `query` came.
    #[must_use]
    pub fn of_query(query: Option<&str>) -> Self {
        let sent_away =
            query.is_some_and(|query| query.split('&').any(|pair| pair == SENT_AWAY_QUERY));
        if sent_away {
            Self::SentAway
        } else {
            Self::First
        }
    }
}

/// `url` with [`SENT_AWAY_QUERY`] in its query.
fn sent_away_url(url: &str) -> String {
    let (before, fragment) = url
        .split_once('#')
        .map_or((url, None), |(before, fragment)| (before, Some(fragment)));
    let joint = if before.contains('?') { '&' } else { '?' };
    let mut sent = format!("{before}{joint}{SENT_AWAY_QUERY}");
    if let Some(fragment) = fragment {
        sent.push('#');
        sent.push_str(fragment);
    }
    sent
}

/// The request streams of a session that are read at once, for a session
/// that may have `peer_requests` open.
///
/// WHY above the session's limit on open requests: a request over that
/// limit has to be read to be answered with `EXCESSIVE_LOAD`. With fewer
/// readers than requests it would wait unanswered instead.
fn request_streams_for(peer_requests: usize) -> usize {
    peer_requests.saturating_add(8)
}

impl HubConfig {
    /// [`Self::streams`] with as many request streams read at once as
    /// [`Self::limits_of_a_session`] lets a session have open
    /// ([`request_streams_for`]): what the hub reads with, and what the QUIC
    /// limits of the relay's connections follow.
    #[must_use]
    pub fn stream_config(&self) -> StreamConfig {
        StreamConfig {
            max_request_streams: request_streams_for(self.limits_of_a_session.peer_requests),
            ..self.streams.clone()
        }
    }
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            streams: StreamConfig {
                // The relay counts an Object as received from its first
                // byte (§5.1.2): `Event::ObjectBegun`.
                report_objects_begun: true,
                ..StreamConfig::default()
            },
            max_sessions: 1_000,
            when_full: None,
            event_queue_len: 1024,
            session_queue_len: 256,
            send: SendLimits {
                bytes: 8 * 1024 * 1024,
                objects: 16_384,
                streams: 4_096,
                delivery_patience: None,
            },
            send_for_one_subscription: SendLimits {
                bytes: 4 * 1024 * 1024,
                objects: 4_096,
                streams: 1_024,
                delivery_patience: None,
            },
            send_for_the_relay_bytes: 2 * 1024 * 1024 * 1024,
            drain_timeout: Duration::from_secs(2),
            slow_open: Duration::from_millis(10),
            auth_key: None,
            auth_setup: true,
            early_stream_timeout: Duration::from_secs(2),
            request_close_timeout: Duration::from_secs(2),
            closed_read_timeout: Duration::from_secs(1),
            max_timers_per_session: 1024,
            announcement_every_ms: None,
            announcements_at_once: None,
            stats_every: None,
            log_arrivals: None,
            cached_fetch_whole_ms: 30_000,
            cached_fetch_least_kbps: 500,
            held_beyond_cache_bytes: None,
            limits_of_a_session: LimitsOfASession::default(),
            requests_for_one_subscriber: None,
            fetches_for_one_subscriber: None,
            upstream_timeout: Duration::from_millis(xmoq_relay_core::UPSTREAM_ANSWER_WAIT_MS),
            setup_timeout: Duration::from_secs(10),
            idle_session_timeout: Duration::from_secs(30),
            implementation: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))
                .to_string(),
            log: None,
        }
    }
}

/// Takes log lines without ever making the caller wait.
#[derive(Clone)]
pub struct Log {
    lines: mpsc::Sender<String>,
    /// Lines that did not fit in the queue since the last one that did.
    dropped: Arc<AtomicU64>,
}

impl Log {
    /// A log whose lines a test reads.
    #[cfg(test)]
    pub fn collect() -> (Self, mpsc::Receiver<String>) {
        let (lines, collected) = mpsc::channel(1024);
        let log = Self {
            lines,
            dropped: Arc::new(AtomicU64::new(0)),
        };
        (log, collected)
    }

    /// A log that prints to stdout, 1,024 lines at most waiting.
    #[must_use]
    pub fn stdout() -> Self {
        Self::writing_to(std::io::stdout(), 1024)
    }

    /// A log that prints to stderr, `queue` lines at most waiting.
    #[must_use]
    pub fn stderr(queue: usize) -> Self {
        Self::writing_to(std::io::stderr(), queue)
    }

    /// A log whose lines a thread of its own writes to `out`.
    ///
    /// WHY a thread: the hub is one task for all sessions, and a write to an
    /// output that nobody reads would stop it, and all sessions with it; on
    /// a task of the runtime, it would stop a worker of it. Lines that do not
    /// fit in the queue are dropped, and their number is printed when there
    /// is room again. A write that fails ends the thread, and the lines after
    /// it go nowhere, as they would.
    fn writing_to(out: impl std::io::Write + Send + 'static, queue: usize) -> Self {
        let (lines, mut queued) = mpsc::channel::<String>(queue);
        std::thread::spawn(move || {
            let mut out = out;
            while let Some(line) = queued.blocking_recv() {
                if writeln!(out, "{line}").is_err() {
                    break;
                }
            }
        });
        Self {
            lines,
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Queues a line. A line that does not fit is dropped: the log is not
    /// worth a stalled relay.
    pub fn print(&self, line: String) {
        // Relaxed: a count for a log line.
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            let notice = log_dropped_line(dropped);
            if self.lines.try_send(notice).is_err() {
                self.dropped.fetch_add(dropped, Ordering::Relaxed);
            }
        }
        if self.lines.try_send(line).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// `interop/harness.ts` looks for it.
fn log_dropped_line(dropped: u64) -> String {
    format!("LOG_DROPPED lines={dropped}")
}

/// What reaches the hub.
enum Envelope<C: Connection> {
    NewSession {
        session: SessionKey,
        connection: Arc<C>,
        arrival: Arrival,
    },
    /// The Timeout of the GOAWAY of a session that was sent away is over.
    SendAwayDue {
        session: SessionKey,
    },
    /// The first message of a request stream, with the half to answer on.
    RequestOpened {
        session: SessionKey,
        stream: StreamKey,
        send: C::Send,
        first: ControlMessage,
    },
    Core(Event),
    /// What came of [`Action::ConnectParent`].
    Parent(Result<Arc<C>, String>),
    /// The session cannot go on, for a reason found on the I/O side.
    Broken {
        session: SessionKey,
        code: TerminationCode,
        reason: &'static str,
    },
    /// The time a client has for its SETUP is over
    /// ([`HubConfig::setup_timeout`]).
    SetupDue {
        session: SessionKey,
    },
    /// The relay is to stop.
    Stop(Stopping),
    /// A test asks what the hub holds.
    #[cfg(test)]
    Inspect(tokio::sync::oneshot::Sender<Held>),
}

/// How the relay is to stop.
enum Stopping {
    /// Every session is told with GOAWAY, and `drained` is answered once no
    /// session is left.
    GoAway {
        going_away: GoingAway,
        drained: oneshot::Sender<()>,
    },
    /// Every session is closed with `code`, and `answer` is answered when
    /// the closes have reached the peers.
    Close {
        answer: oneshot::Sender<()>,
        code: u32,
        reason: &'static str,
    },
}

/// What the hub and its relay hold.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    /// Sessions the hub has handles for.
    pub sessions: usize,
    /// Writers of request streams, over all sessions.
    pub request_writers: usize,
    /// Writers of subgroup streams that are not done.
    pub out_streams: usize,
    /// Tasks of request streams the relay opened, over all sessions.
    pub own_requests: usize,
    /// Fetch streams that are being passed on.
    pub fetch_pipes: usize,
    /// Timers that are running.
    pub timers: usize,
    /// What the relay holds.
    pub relay: xmoq_relay_core::Counts,
}

/// How the hub ends the task of a request stream the relay opened.
#[derive(Clone, Copy)]
enum OwnRequestEnding {
    /// With a FIN, after the messages that wait.
    Finish,
    /// With a reset of the code, ahead of what waits.
    Cancel(u32),
}

/// The task of a request stream the relay opened, as the hub holds it.
///
/// WHY the ending apart from the messages: in one queue, a `Cancel` would
/// wait behind updates the peer takes no credit for, and a `Finish` would
/// get in only while the session keeps the updates that wait below the
/// queue's room (`MAX_UNANSWERED_UPDATES`, which it does today); one that
/// did not get in would turn into the reset of a session that is going.
struct OwnRequestTask {
    /// Messages after the request: the `REQUEST_UPDATE`s of a SUBSCRIBE.
    updates: mpsc::Sender<ControlMessage>,
    ending: oneshot::Sender<OwnRequestEnding>,
}

/// What the hub asks the writer of one request stream to do.
enum RequestCommand {
    Send(ControlMessage),
    Finish,
}

/// The writer of the relay's half of a request stream the peer opened.
struct RequestWriter {
    commands: mpsc::Sender<RequestCommand>,
    /// Resets the stream with the code, ahead of what waits in `commands`
    /// and of a message being written. WHY not in `commands`: a stream the
    /// peer gives no credit for would hold the reset back until the session
    /// ends, while the core has the request over (§3.3.2).
    reset: oneshot::Sender<u32>,
}

struct SessionHandle<C: Connection> {
    connection: Arc<C>,
    control: mpsc::Sender<ControlMessage>,
    /// A message came on the client's control stream: its SETUP, or what
    /// the relay closes the session for.
    said_something: bool,
    /// When the time the session may hold nothing is over, as it is in
    /// [`Hub::idle`]; `None` while it holds something, or has sent no SETUP.
    idle_until: Option<Instant>,
    /// The writers of the request streams the peer has open.
    requests: HashMap<StreamKey, RequestWriter>,
    /// The tasks of the request streams the relay has open on the session.
    own_requests: HashMap<OwnRequestKey, OwnRequestTask>,
    stops: StreamStops,
    /// What waits to be written to this session's subgroup streams.
    budget: Arc<SendBudget>,
    /// What waits to be written to each of its subscriptions, by the Track
    /// Alias the relay gave it, which a session never gets again; within
    /// [`Self::budget`]. One that no stream holds is dropped when the next
    /// stream opens.
    subscriptions: HashMap<u64, SubscriptionBudget>,
    /// The subgroup streams towards it that the relay was told to open and
    /// that are not open yet: they wait for the peer to allow one more
    /// stream. For the log (`DATA_OPENED`).
    opening: Arc<AtomicUsize>,
    /// The writers of its control and request streams, and the task that owns
    /// its readers.
    tasks: Vec<JoinHandle<()>>,
    /// The tasks that write a fetch stream the relay made itself and are not
    /// done.
    fetch_writes: Vec<JoinHandle<()>>,
    /// What stops such a task and what reorders its stream, by the request
    /// stream of its FETCH ([`Action::AbortFetchWrite`],
    /// [`Action::SetFetchSendOrder`]).
    fetch_write_controls: HashMap<StreamKey, FetchControl>,
    /// It came when the relay was full and was sent to
    /// [`HubConfig::when_full`]; it counts for [`MAX_SENDING_AWAY`] and not
    /// for [`HubConfig::max_sessions`].
    sent_away: bool,
    /// The relay's: [`Role::Client`] for its session with the parent.
    role: Role,
}

/// What waits to be written to one subscription, and the stream of it that
/// was opened last, which names the subscription to the core
/// ([`SendLimit::Relay`]).
struct SubscriptionBudget {
    budget: Arc<SendBudget>,
    latest: OutStreamKey,
}

/// The path of a connection as fields of a log line; nothing for a
/// connection that does not say.
fn path_now<C: Connection>(connection: &C) -> String {
    connection.path().map_or_else(String::new, |path| {
        format!(
            " cwnd={} rtt_ms={} lost_packets={} congestion_events={}",
            path.cwnd,
            path.rtt.as_millis(),
            path.lost_packets,
            path.congestion_events
        )
    })
}

/// How long the streams of a session the relay closed are kept as they are,
/// for the close to reach the peer first ([`Hub::close`]). The library
/// closes the connection when the peer has answered the capsule, or after
/// max(3 x RTT, 100 ms) (web-transport-quinn 0.13.2, `Session::close`), so
/// this is a bound for a connection that does not end.
const CLOSE_REACHES_THE_PEER: Duration = Duration::from_secs(2);

/// [`QueueFull::level`] of a stream's own budget, its subscription's; 1 is
/// its session's. The relay's refuses nothing.
const SUBSCRIPTION_BUDGET: usize = 0;

/// `NO_ERROR`: "The session is being terminated without an error" (§3.5).
const NO_ERROR: u32 = 0x0;

impl<C: Connection> Drop for SessionHandle<C> {
    fn drop(&mut self) {
        for task in self.tasks.iter().chain(&self.fetch_writes) {
            task.abort();
        }
    }
}

/// What the hub holds of a fetch stream it passes on or writes: the way to
/// cut it short, and the way to change where it stands among the streams to
/// the subscriber ([`Action::SetFetchSendOrder`]).
struct FetchControl {
    abort: oneshot::Sender<u32>,
    send_order: watch::Sender<i32>,
}

impl FetchControl {
    /// The control, and the two ends its task reads.
    fn new(send_order: i32) -> (Self, oneshot::Receiver<u32>, watch::Receiver<i32>) {
        let (abort, aborted) = oneshot::channel();
        let (send_order, reordered) = watch::channel(send_order);
        (Self { abort, send_order }, aborted, reordered)
    }
}

/// Gives a fetch stream the send order the relay changed it to since it was
/// last looked at ([`Action::SetFetchSendOrder`]).
///
/// WHY between writes, and not while one waits: the write holds the
/// writer, and a write that is dropped to make room may have written a part
/// of its bytes. A write waits for room as long as the subscriber's wait at
/// most, and the pieces of a stream that is passed on are 1 KiB.
fn follow_send_order<S: SendStream>(
    reordered: &mut watch::Receiver<i32>,
    writer: &mut FetchStreamWriter<S>,
) {
    if reordered.has_changed().unwrap_or(false) {
        writer.set_send_order(*reordered.borrow_and_update());
    }
}

/// A fetch stream the relay writes itself ([`Action::WriteFetchStream`]).
struct FetchWrite<C> {
    connection: Arc<C>,
    header: FetchHeader,
    /// Where the stream stands among the streams to the subscriber
    /// (`Action::WriteFetchStream`), and where the relay moves it to later.
    send_order: watch::Receiver<i32>,
    pieces: Vec<FetchPiece>,
    /// How long the subscriber may take nothing.
    patience: Duration,
    /// How long the subscriber may take for the whole stream
    /// ([`HubConfig::cached_fetch_whole`]).
    whole: Duration,
    trace: Trace,
    label: String,
}

impl<C: Connection> FetchWrite<C> {
    /// Opens the stream, writes the pieces one after the other and finishes
    /// it. `None` when that was done; else the Stream Reset Error Code the
    /// stream was given up with, which what was opened of it was reset with.
    ///
    /// A subscriber that takes nothing for `patience`, of the stream itself
    /// or of one piece, is told so with the code of a fetch stream that is
    /// passed on and not taken; one that stops the stream, or is gone, with
    /// `CANCELLED`; and what comes on `aborted` ends the stream with the
    /// code that came, whenever it comes before the stream is finished.
    ///
    /// WHY piece by piece, and the wait anew for each: the pieces are the
    /// Objects of a Group, megabytes together, and a subscriber that takes
    /// them slowly is not one that takes nothing.
    ///
    /// The whole has `whole` all the same, from when the stream is asked
    /// for: a subscriber that takes each piece just in time would otherwise
    /// keep the Objects of a Group the cache has moved on from for hours
    /// (4,096 Objects, 10 seconds each), and 16 such streams of a session
    /// as many Groups.
    ///
    /// WHY an Object is copied to be encoded, one at a time: the cache
    /// shares it with the subgroup streams, and a fetch stream encodes it
    /// with other fields before it (§11.4.4.1).
    ///
    /// Not verified against a real connection: giving up while the
    /// `FETCH_HEADER` is not all written is tested on an in-memory
    /// connection whose streams hold one byte, and on no real one, where the
    /// wait is more likely inside the opening of the stream, with nothing to
    /// reset. The clocks of the opening and of each piece run in tests on
    /// in-memory connections only, and so does the stop of such a stream
    /// when its FETCH is cancelled or its track found malformed: the stream
    /// is a few bytes, which a real transport takes before a cancellation
    /// can arrive. A reset while a write waits is tested over loopback as
    /// well (`xmoq-moqt-io`).
    async fn run(self, mut aborted: oneshot::Receiver<u32>) -> Written {
        let ended = |how, pieces| Written { how, pieces };
        // WHY not simply added: a time for the whole that somebody set to
        // more than a clock can hold would panic here. A year is as good as
        // none.
        let now = tokio::time::Instant::now();
        let by = now
            .checked_add(self.whole)
            .unwrap_or_else(|| now + Duration::from_hours(8760));
        let mut orders = Orders {
            aborted: &mut aborted,
            open: true,
        };
        let mut reordered = self.send_order.clone();
        // The stream itself. WHY it is opened without its header: an order
        // to stop that comes while the header waits to be written has to
        // reset the stream with its own code, and a call that opens and
        // writes in one resets with a code that is chosen before.
        let opening = open_fetch_stream_without_header(&*self.connection, &self.trace, &self.label);
        let opened = tokio::select! {
            biased;
            code = orders.next() => return ended(WriteEnd::Aborted(code), 0),
            opened = tokio::time::timeout(self.patience, opening) => opened,
        };
        let mut writer = match opened {
            Ok(Ok(writer)) => writer,
            // Nothing is there to reset.
            Ok(Err(_)) => return ended(WriteEnd::Stopped, 0),
            Err(_) => return ended(WriteEnd::NotTaken, 0),
        };
        // Before the header: §7.1 has the Objects of a FETCH scheduled with
        // those of the subscriptions.
        writer.set_send_order(*reordered.borrow_and_update());
        // Objects of one Group, or indicators alone: the same bytes in
        // either Group Order.
        let mut encoder = FetchStreamEncoder::new(self.header, GroupOrder::Ascending);
        let header = encoder.take_bytes();
        if let Some(how) = self.write(&mut writer, &header, &mut orders, by).await {
            writer.reset(how.code());
            return ended(how, 0);
        }
        for (written, piece) in self.pieces.iter().enumerate() {
            match piece {
                FetchPiece::Object(cached) => {
                    let object = FetchObject {
                        group_id: cached.group_id,
                        subgroup_id: Some(cached.subgroup_id),
                        object_id: cached.object.object_id,
                        publisher_priority: cached.publisher_priority,
                        properties: cached.object.properties.clone(),
                        payload: cached.object.payload.clone(),
                    };
                    // One the encoder refuses is one the relay put together
                    // wrongly; the stream cannot go on without it.
                    if encoder.object(&object).is_err() {
                        writer.reset(WriteEnd::Failed.code());
                        return ended(WriteEnd::Failed, written);
                    }
                }
                FetchPiece::EndOfRange(kind, location) => encoder.end_of_range(*kind, *location),
            }
            let bytes = encoder.take_bytes();
            follow_send_order(&mut reordered, &mut writer);
            if let Some(how) = self.write(&mut writer, &bytes, &mut orders, by).await {
                writer.reset(how.code());
                return ended(how, written);
            }
        }
        // An order that came with the last write.
        if let Some(code) = orders.there() {
            writer.reset(code);
            return ended(WriteEnd::Aborted(code), self.pieces.len());
        }
        writer.finish();
        ended(WriteEnd::Finished, self.pieces.len())
    }

    /// Writes one piece, or the header. `None` when it is written; else why
    /// the stream is given up.
    async fn write(
        &self,
        writer: &mut FetchStreamWriter<C::Send>,
        bytes: &[u8],
        orders: &mut Orders<'_>,
        by: tokio::time::Instant,
    ) -> Option<WriteEnd> {
        // WHY one write that is polled again, and not a write anew after an
        // order that was none: a write that is dropped may have written a
        // part of the piece, and a second one would write that part again.
        let write = tokio::time::timeout(self.patience, writer.write(bytes));
        tokio::pin!(write);
        tokio::select! {
            // WHY the order first: a write that can complete at once would
            // otherwise be taken as often as an order that is there
            // already.
            biased;
            code = orders.next() => Some(WriteEnd::Aborted(code)),
            () = tokio::time::sleep_until(by) => Some(WriteEnd::TooSlow),
            written = &mut write => match written {
                Ok(Ok(())) => None,
                Ok(Err(_)) => Some(WriteEnd::Stopped),
                Err(_) => Some(WriteEnd::NotTaken),
            },
        }
    }
}

/// How the writing of a fetch stream of the relay's own ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteEnd {
    /// Everything was written, and the stream finished.
    Finished,
    /// The subscriber took nothing for the time it had.
    NotTaken,
    /// The subscriber did not take the whole stream in the time it had.
    TooSlow,
    /// The subscriber stopped the stream, or the connection is gone.
    Stopped,
    /// The relay ordered it stopped, with this code
    /// ([`Action::AbortFetchWrite`]).
    Aborted(u32),
    /// The relay put something together that a fetch stream cannot carry,
    /// or the task that wrote failed.
    Failed,
}

impl WriteEnd {
    /// The Stream Reset Error Code the stream is reset with, and its
    /// request stream after it. Not asked of a stream that was finished.
    fn code(self) -> u32 {
        match self {
            Self::Finished | Self::Stopped => stream_reset_code::CANCELLED,
            // WHY the same code for both: either way the subscriber is
            // behind what the relay keeps for it. §3.3.3 words the code for
            // a subscription; the notes have why it is used for a FETCH
            // (row on a fetch stream that is not taken).
            Self::NotTaken | Self::TooSlow => stream_reset_code::TOO_FAR_BEHIND,
            Self::Aborted(code) => code,
            Self::Failed => stream_reset_code::INTERNAL_ERROR,
        }
    }
}

/// What became of a fetch stream the relay wrote itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Written {
    how: WriteEnd,
    /// The pieces that were written whole.
    pieces: usize,
}

impl Written {
    /// `None` for a stream that was written and finished; else the code it
    /// was given up with ([`Event::FetchStreamWritten`]).
    fn given_up(self) -> Option<u32> {
        (self.how != WriteEnd::Finished).then(|| self.how.code())
    }

    /// The line of the log: how the writing ended, how many pieces were
    /// written whole, and the code of the reset.
    fn line(self, session: SessionKey, stream: StreamKey) -> String {
        let how = match self.how {
            WriteEnd::Finished => "finished",
            WriteEnd::NotTaken => "not_taken",
            WriteEnd::TooSlow => "too_slow",
            WriteEnd::Stopped => "stopped",
            WriteEnd::Aborted(_) => "aborted",
            WriteEnd::Failed => "failed",
        };
        let code = self
            .given_up()
            .map_or(String::new(), |code| format!(" code={code:#x}"));
        format!(
            "FETCH_WRITE_ENDED session={} stream={} {how} pieces={}{code}",
            session.0, stream.0, self.pieces
        )
    }
}

/// The orders to stop a fetch stream the relay writes
/// ([`Action::AbortFetchWrite`]): one at most.
struct Orders<'a> {
    aborted: &'a mut oneshot::Receiver<u32>,
    /// Whether one can still come. WHY kept: a receiver that has completed
    /// is not asked again.
    open: bool,
}

impl Orders<'_> {
    /// The order, with its Stream Reset Error Code. Never completes when
    /// none comes any more.
    async fn next(&mut self) -> u32 {
        if self.open {
            // Dropped while it waits, the call has taken nothing.
            let order = (&mut *self.aborted).await;
            self.open = false;
            if let Ok(code) = order {
                return code;
            }
        }
        std::future::pending().await
    }

    /// The order that is there already.
    fn there(&mut self) -> Option<u32> {
        if !self.open {
            return None;
        }
        let order = self.aborted.try_recv().ok();
        self.open = order.is_none();
        order
    }
}

/// Runs `work` as a task of its own and waits for it: `Err` where it
/// panicked. The task ends when this call is dropped.
///
/// WHY a task of its own: the caller has to tell the relay how the work
/// ended whatever became of it, a panic included. WHY it ends with the call:
/// the caller is a task that goes with its session, and work that outlived
/// it would keep the connection it holds open, with nobody left to stop it.
async fn in_a_task_that_ends_with_this_call<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> Result<T, tokio::task::JoinError> {
    struct EndsWithIt<T>(JoinHandle<T>);
    impl<T> Drop for EndsWithIt<T> {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let mut task = EndsWithIt(tokio::spawn(work));
    (&mut task.0).await
}

/// Adds sessions to a running hub.
pub struct HubHandle<C: Connection> {
    events: mpsc::Sender<Envelope<C>>,
    next_session: Arc<AtomicU64>,
}

impl<C: Connection> Clone for HubHandle<C> {
    fn clone(&self) -> Self {
        Self {
            events: self.events.clone(),
            next_session: Arc::clone(&self.next_session),
        }
    }
}

impl<C: Connection> HubHandle<C> {
    /// What the hub holds, once it has dealt with everything queued before.
    #[cfg(test)]
    pub async fn held(&self) -> Held {
        let (answer, held) = tokio::sync::oneshot::channel();
        let asked = self.events.send(Envelope::Inspect(answer)).await;
        assert!(asked.is_ok(), "the hub is running");
        held.await.expect("the hub answers")
    }

    /// Hands the hub an event as an I/O task of its own would: what the
    /// peers of a test cannot be made to bring about.
    #[cfg(test)]
    pub async fn inject(&self, event: Event) {
        let sent = self.events.send(Envelope::Core(event)).await;
        assert!(sent.is_ok(), "the hub is running");
    }

    /// Closes every session with `code` and completes when the peers have
    /// been told, or [`CLOSE_REACHES_THE_PEER`] later. For a relay that is
    /// about to end: `GOAWAY_TIMEOUT` after a GOAWAY whose time is over
    /// ([`Self::go_away`]; §3.6: "The sender closes the session with a
    /// `GOAWAY_TIMEOUT` if the peer doesn't close the session within the
    /// indicated Timeout").
    ///
    /// WHY the sessions are closed and not left to the end of the process: a
    /// connection that merely stops answering is found out by the peer's
    /// idle timeout, seconds later, and without a code.
    pub async fn stop_with(&self, code: u32, reason: &'static str) {
        let (answer, stopped) = oneshot::channel();
        let stop = Envelope::Stop(Stopping::Close {
            answer,
            code,
            reason,
        });
        if self.events.send(stop).await.is_ok() {
            let _ = stopped.await;
        }
    }

    /// Tells every session with GOAWAY that the relay is to stop (§3.6),
    /// and completes once no session is left; the caller sees to the
    /// Timeout.
    pub async fn go_away(&self, new_session_uri: Vec<u8>, timeout_ms: u64) {
        let (drained, gone) = oneshot::channel();
        let going_away = GoingAway {
            new_session_uri,
            timeout_ms,
        };
        let go_away = Envelope::Stop(Stopping::GoAway {
            going_away,
            drained,
        });
        if self.events.send(go_away).await.is_ok() {
            let _ = gone.await;
        }
    }

    /// Hands a connection whose handshake is done to the relay.
    pub async fn add_session(&self, connection: C, arrival: Arrival) {
        // Relaxed: the numbers only have to differ.
        let session = SessionKey(self.next_session.fetch_add(1, Ordering::Relaxed));
        let envelope = Envelope::NewSession {
            session,
            connection: Arc::new(connection),
            arrival,
        };
        // An error means the hub is gone, and the connection with the envelope.
        let _ = self.events.send(envelope).await;
    }
}

/// Makes a connection to the parent relay, or says why it could not: to
/// the URI it is given (the New Session URI of a GOAWAY of the parent,
/// §10.4), or to the place it was made for where it is given none.
pub type Connect<C> = Arc<
    dyn Fn(Option<String>) -> Pin<Box<dyn Future<Output = Result<C, String>> + Send>> + Send + Sync,
>;

/// The relay above this one ([`spawn_with_parent`]).
pub struct Parent<C> {
    /// The tracks it is asked for ([`Relay::set_parent`]).
    pub namespace: TrackNamespace,
    /// Called each time a session with it is wanted.
    pub connect: Connect<C>,
}

/// Starts the hub of a relay. The returned task ends only if the hub panics,
/// since handles to it are held by the hub itself; whoever starts the hub
/// watches the task and does not go on accepting sessions without it.
#[cfg(test)]
#[must_use]
pub fn spawn<C: Connection>(config: HubConfig) -> (HubHandle<C>, JoinHandle<()>) {
    spawn_with_parent(config, None)
}

/// Starts the hub of a relay, as `spawn` of the tests does, for a relay that
/// asks `parent` for the tracks none of its own sessions announced.
#[must_use]
pub fn spawn_with_parent<C: Connection>(
    config: HubConfig,
    parent: Option<Parent<C>>,
) -> (HubHandle<C>, JoinHandle<()>) {
    let mut config = config;
    config.streams = config.stream_config();
    let (events, inbox) = mpsc::channel(config.event_queue_len);
    let mut relay = Relay::new(&config.implementation);
    if let Some(parent) = &parent {
        relay.set_parent(parent.namespace.clone());
    }
    if let Some(bytes) = config.held_beyond_cache_bytes {
        relay.set_held_beyond_the_cache(bytes);
    }
    relay.set_share_of_one_subscriber(
        config
            .requests_for_one_subscriber
            .unwrap_or(xmoq_relay_core::MAX_OWN_REQUESTS_FOR_ONE_SUBSCRIBER),
        config
            .fetches_for_one_subscriber
            .unwrap_or(xmoq_relay_core::MAX_OWN_FETCHES_FOR_ONE_SUBSCRIBER),
    );
    relay.set_announcements(
        config
            .announcements_at_once
            .unwrap_or(xmoq_relay_core::MAX_ANNOUNCEMENTS_AT_ONCE),
        config
            .announcement_every_ms
            .unwrap_or(xmoq_relay_core::ANNOUNCEMENT_EVERY_MS),
    );
    relay.set_upstream_answer_wait_ms(
        u64::try_from(config.upstream_timeout.as_millis()).unwrap_or(u64::MAX),
    );
    relay.set_limits_of_a_session(config.limits_of_a_session);
    if let Some(key) = &config.auth_key {
        relay.set_authorizer(crate::auth::RoomTokens::new(key, config.auth_setup));
    }
    let next_session = Arc::new(AtomicU64::new(0));
    // It counts and refuses nothing: the relay's bound is kept in
    // `Hub::keep_to_the_relay_s_bound`.
    let relay_budget = SendBudget::new(SendLimits {
        bytes: usize::MAX,
        objects: usize::MAX,
        streams: usize::MAX,
        delivery_patience: None,
    });
    let hub = Hub {
        relay,
        config,
        parent,
        next_session: Arc::clone(&next_session),
        events: events.clone(),
        sessions: HashMap::new(),
        budget: relay_budget,
        drained: None,
        out_streams: HashMap::new(),
        fetch_pipes: HashMap::new(),
        timers: Timers::default(),
        drains: HashMap::new(),
        idle: BTreeSet::new(),
        closing: std::collections::HashSet::new(),
    };
    let task = tokio::spawn(hub.run(inbox));
    let handle = HubHandle {
        events,
        next_session,
    };
    (handle, task)
}

struct Hub<C: Connection> {
    relay: Relay,
    config: HubConfig,
    parent: Option<Parent<C>>,
    /// Shared with [`HubHandle`]: the hub names its session with the
    /// parent from the same numbers.
    next_session: Arc<AtomicU64>,
    /// For the tasks the hub starts, which report back through it.
    events: mpsc::Sender<Envelope<C>>,
    sessions: HashMap<SessionKey, SessionHandle<C>>,
    /// What waits to be written to all the subscribers of the relay; the
    /// budget of every session is within it. It refuses nothing
    /// ([`Self::keep_to_the_relay_s_bound`]).
    budget: Arc<SendBudget>,
    /// Answered once no session is left, after GOAWAY was sent
    /// ([`HubHandle::go_away`]).
    drained: Option<oneshot::Sender<()>>,
    /// The writers of the subgroup streams that are not done, with the session
    /// each writes to.
    out_streams: HashMap<OutStreamKey, (SessionKey, OutgoingSubgroup)>,
    /// The fetch streams that are being passed on, by the subscriber's
    /// session and the request stream of its FETCH, each with the way to cut
    /// it short and to reorder it. Dropping that cuts it short as well.
    fetch_pipes: HashMap<(SessionKey, StreamKey), FetchControl>,
    timers: Timers,
    /// When each wait for the last streams of a source began, by its session
    /// and number, for the line that says how long it ran (`DRAIN_ENDED`).
    /// WHY kept here and not read from the timer: the interop run compares
    /// the time with the relay's wait, and the driver reads the lines too
    /// late to time them itself (`interop/disconnect.ts`, publish-anew).
    drains: HashMap<(SessionKey, u64), Instant>,
    /// The sessions that hold nothing, by when their time to is over.
    ///
    /// WHY here and not a task that sleeps: a peer decides how often its
    /// session comes to hold nothing (a SUBSCRIBE that is refused, and its
    /// FIN), and a task could not be taken back when the session holds
    /// something again; here a session has one entry at most. WHY NOT in
    /// [`Timers`]: those hand their events to the core and count against
    /// the timers a session may have, and this one is the hub's own.
    idle: BTreeSet<(Instant, SessionKey)>,
    /// The sessions the relay closed itself and has not forgotten yet: their
    /// streams are held by a task until the close has reached the peer
    /// ([`Self::close`]).
    closing: std::collections::HashSet<SessionKey>,
}

/// The timers the relay asked for, in one place.
///
/// WHY not a task per timer: a peer decides how many timers start (every
/// stream it opens ahead of its PUBLISH starts one), and a task cannot be
/// taken back when its session goes. Here a session's timers are counted,
/// bounded, and dropped with the session.
#[derive(Default)]
struct Timers {
    /// By the time they are due, and a number that keeps equal times apart.
    pending: BTreeMap<(Instant, u64), (SessionKey, Event)>,
    per_session: HashMap<SessionKey, usize>,
    /// Where the timer for the answer to a request of the relay is, so that
    /// it can be dropped when the answer came.
    upstream: HashMap<(SessionKey, OwnRequestKey), (Instant, u64)>,
    /// Where the timer for a peer to end its half of a request stream is,
    /// so that it can be dropped when the peer did: the one that runs
    /// while the relay keeps its own half of a served FETCH open for it,
    /// and the one after the relay finished its half. A stream has one of
    /// the two at a time.
    request_close: HashMap<(SessionKey, StreamKey), (Instant, u64)>,
    /// Where the timer of a SUBSCRIBE that is held for a publisher to appear
    /// is, so that it can be dropped when the SUBSCRIBE is answered or
    /// gone. WHY dropped, though it does nothing when it runs out then: a
    /// peer chooses how long it runs, and it counts among the timers its
    /// session may have.
    rendezvous: HashMap<(SessionKey, StreamKey), (Instant, u64)>,
    /// Where the timer of a wait for the last streams of a source is, by
    /// the number of the wait, so that it can be dropped when the source
    /// is over before it.
    drain: HashMap<(SessionKey, u64), (Instant, u64)>,
    /// Where the timer of a stream held for its Track Alias is, and that of
    /// a Joining FETCH held for its SUBSCRIBE to come, so that they can be
    /// dropped when what they waited for is settled.
    early: HashMap<(SessionKey, StreamKey), (Instant, u64)>,
    joining: HashMap<(SessionKey, StreamKey), (Instant, u64)>,
    /// Where the timer of a FETCH held for an Object the relay is
    /// receiving is, so that it can be dropped when the FETCH is let go of.
    receipt: HashMap<(SessionKey, StreamKey), (Instant, u64)>,
    next: u64,
}

impl Timers {
    /// Starts a timer and returns `None`, unless the session already has
    /// `limit` of them running; then the event is handed back.
    fn start(
        &mut self,
        session: SessionKey,
        limit: usize,
        wait: Duration,
        event: Event,
    ) -> Option<Event> {
        // A new wait for the same request replaces the one that runs, and
        // takes its place in the count.
        if let Event::UpstreamTimeout { session, request } = event {
            self.stop_upstream(session, request);
        }
        let running = self.per_session.entry(session).or_default();
        if *running >= limit {
            return Some(event);
        }
        *running += 1;
        let at = (Instant::now() + wait, self.next);
        match event {
            Event::UpstreamTimeout { session, request } => {
                self.upstream.insert((session, request), at);
            }
            Event::RequestTimeout { session, stream }
            | Event::FetchEndTimeout { session, stream } => {
                self.request_close.insert((session, stream), at);
            }
            Event::RendezvousTimeout { session, stream } => {
                self.rendezvous.insert((session, stream), at);
            }
            Event::DrainTimeout { session, wait, .. } => {
                self.drain.insert((session, wait), at);
            }
            Event::EarlyStreamTimeout { session, stream } => {
                self.early.insert((session, stream), at);
            }
            Event::JoiningFetchTimeout { session, stream } => {
                self.joining.insert((session, stream), at);
            }
            Event::ReceiptTimeout { session, stream } => {
                self.receipt.insert((session, stream), at);
            }
            _ => {}
        }
        self.pending.insert(at, (session, event));
        self.next = self.next.wrapping_add(1);
        None
    }

    /// Drops the timer for the answer to a request of the relay.
    fn stop_upstream(&mut self, session: SessionKey, request: OwnRequestKey) {
        let Some(at) = self.upstream.remove(&(session, request)) else {
            return;
        };
        if self.pending.remove(&at).is_some()
            && let Some(running) = self.per_session.get_mut(&session)
        {
            *running = running.saturating_sub(1);
        }
    }

    /// Drops the timer for a peer to end its half of a request stream,
    /// whichever of the two it is.
    ///
    /// WHY dropped and not left to run out: every request the relay refuses
    /// or serves to its end starts one, and a peer that ends its streams as
    /// it should would hold as many timers as it made requests in that
    /// time, against a limit that is there for peers that do not.
    /// Drops the timer of a SUBSCRIBE that was held for a publisher to
    /// appear: the SUBSCRIBE was answered, or its stream is gone.
    fn stop_rendezvous(&mut self, session: SessionKey, stream: StreamKey) {
        let Some(at) = self.rendezvous.remove(&(session, stream)) else {
            return;
        };
        if self.pending.remove(&at).is_some()
            && let Some(running) = self.per_session.get_mut(&session)
        {
            *running = running.saturating_sub(1);
        }
    }

    /// Drops the timer of a wait for the last streams of a source.
    fn stop_drain(&mut self, session: SessionKey, wait: u64) {
        let Some(at) = self.drain.remove(&(session, wait)) else {
            return;
        };
        if self.pending.remove(&at).is_some()
            && let Some(running) = self.per_session.get_mut(&session)
        {
            *running = running.saturating_sub(1);
        }
    }

    /// Drops the timer of a stream held for its Track Alias.
    fn stop_early(&mut self, session: SessionKey, stream: StreamKey) {
        let at = self.early.remove(&(session, stream));
        self.drop_pending(session, at);
    }

    /// Drops the timer of a Joining FETCH held for its SUBSCRIBE to come.
    fn stop_joining(&mut self, session: SessionKey, stream: StreamKey) {
        let at = self.joining.remove(&(session, stream));
        self.drop_pending(session, at);
    }

    /// Drops the timer of a FETCH held for an Object the relay is receiving.
    fn stop_receipt(&mut self, session: SessionKey, stream: StreamKey) {
        let at = self.receipt.remove(&(session, stream));
        self.drop_pending(session, at);
    }

    /// Drops the timer due `at`, where there is one, and counts it out of
    /// its session's.
    fn drop_pending(&mut self, session: SessionKey, at: Option<(Instant, u64)>) {
        if let Some(at) = at
            && self.pending.remove(&at).is_some()
            && let Some(running) = self.per_session.get_mut(&session)
        {
            *running = running.saturating_sub(1);
        }
    }

    fn stop_request_close(&mut self, session: SessionKey, stream: StreamKey) {
        let Some(at) = self.request_close.remove(&(session, stream)) else {
            return;
        };
        if self.pending.remove(&at).is_some()
            && let Some(running) = self.per_session.get_mut(&session)
        {
            *running = running.saturating_sub(1);
        }
    }

    /// Completes when the next timer is due; never, when none is running.
    async fn due(&self) {
        match self.pending.first_key_value() {
            Some(((at, _), _)) => tokio::time::sleep_until(*at).await,
            None => std::future::pending().await,
        }
    }

    fn take_due(&mut self) -> VecDeque<Event> {
        let now = Instant::now();
        let mut events = VecDeque::new();
        while let Some(entry) = self.pending.first_entry() {
            if entry.key().0 > now {
                break;
            }
            let (session, event) = entry.remove();
            if let Some(running) = self.per_session.get_mut(&session) {
                *running = running.saturating_sub(1);
            }
            match event {
                Event::UpstreamTimeout { session, request } => {
                    self.upstream.remove(&(session, request));
                }
                Event::RequestTimeout { session, stream }
                | Event::FetchEndTimeout { session, stream } => {
                    self.request_close.remove(&(session, stream));
                }
                Event::RendezvousTimeout { session, stream } => {
                    self.rendezvous.remove(&(session, stream));
                }
                Event::DrainTimeout { session, wait, .. } => {
                    self.drain.remove(&(session, wait));
                }
                Event::EarlyStreamTimeout { session, stream } => {
                    self.early.remove(&(session, stream));
                }
                Event::JoiningFetchTimeout { session, stream } => {
                    self.joining.remove(&(session, stream));
                }
                Event::ReceiptTimeout { session, stream } => {
                    self.receipt.remove(&(session, stream));
                }
                _ => {}
            }
            events.push_back(event);
        }
        events
    }

    fn forget_session(&mut self, session: SessionKey) {
        self.per_session.remove(&session);
        self.pending.retain(|_, (owner, _)| *owner != session);
        self.upstream.retain(|(owner, _), _| *owner != session);
        self.request_close.retain(|(owner, _), _| *owner != session);
        self.rendezvous.retain(|(owner, _), _| *owner != session);
        self.drain.retain(|(owner, _), _| *owner != session);
        self.early.retain(|(owner, _), _| *owner != session);
        self.joining.retain(|(owner, _), _| *owner != session);
        self.receipt.retain(|(owner, _), _| *owner != session);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pending.len()
    }

    /// The timers that are remembered so that they can be dropped early.
    #[cfg(test)]
    fn remembered(&self) -> usize {
        self.upstream.len() + self.request_close.len()
    }
}

impl<C: Connection> Hub<C> {
    async fn run(mut self, mut inbox: mpsc::Receiver<Envelope<C>>) {
        let mut stats = self.config.stats_every.map(tokio::time::interval);
        loop {
            let envelope = tokio::select! {
                envelope = inbox.recv() => envelope,
                _ = async {
                    match stats.as_mut() {
                        Some(stats) => stats.tick().await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.log_stats();
                    continue;
                }
                () = self.timers.due() => {
                    let events = self.timers.take_due();
                    self.feed(events);
                    continue;
                }
                () = async {
                    match self.idle.first() {
                        Some(&(until, _)) => tokio::time::sleep_until(until).await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.idle_due();
                    continue;
                }
            };
            let Some(envelope) = envelope else { return };
            match envelope {
                Envelope::NewSession {
                    session,
                    connection,
                    arrival,
                } => self.add_session(session, connection, Role::Server, arrival),
                Envelope::SendAwayDue { session } => self.send_away_due(session),
                Envelope::Parent(connected) => self.parent_connected(connected),
                Envelope::RequestOpened {
                    session,
                    stream,
                    send,
                    first,
                } => {
                    self.add_request(session, stream, send);
                    self.feed(VecDeque::from([Event::RequestOpened {
                        session,
                        stream,
                        first,
                    }]));
                }
                Envelope::Core(event) => {
                    if let Some(arrivals) = &self.config.log_arrivals {
                        log_arrival(arrivals, &event);
                    }
                    if let Event::ControlMessage { session, .. } = &event
                        && let Some(handle) = self.sessions.get_mut(session)
                    {
                        handle.said_something = true;
                    }
                    match &event {
                        // The peer ended the stream, or stopped the relay's
                        // direction of it; either way the peer's direction is
                        // over (§3.3.2). What becomes of the relay's is for the
                        // relay to say, with FinishRequest or ResetRequest.
                        Event::RequestClosed {
                            session,
                            stream,
                            how: RequestEnd::Aborted,
                        } => {
                            if let Some(handle) = self.sessions.get(session) {
                                handle.stops.stop(stream.0, stream_reset_code::CANCELLED);
                            }
                        }
                        Event::OutStreamClosed { out_stream } => {
                            self.out_streams.remove(out_stream);
                        }
                        _ => {}
                    }
                    self.feed(VecDeque::from([event]));
                }
                Envelope::Broken {
                    session,
                    code,
                    reason,
                } => {
                    let mut pending = VecDeque::new();
                    self.close(session, code.code(), reason, &mut pending);
                    self.feed(pending);
                }
                Envelope::SetupDue { session } => self.setup_due(session),
                Envelope::Stop(stopping) => self.stop(stopping),
                #[cfg(test)]
                Envelope::Inspect(answer) => {
                    let _ = answer.send(self.held());
                }
            }
        }
    }

    #[cfg(test)]
    fn held(&self) -> Held {
        Held {
            sessions: self.sessions.len(),
            request_writers: self
                .sessions
                .values()
                .map(|handle| handle.requests.len())
                .sum(),
            out_streams: self.out_streams.len(),
            own_requests: self
                .sessions
                .values()
                .map(|handle| handle.own_requests.len())
                .sum(),
            fetch_pipes: self.fetch_pipes.len(),
            timers: self.timers.len(),
            relay: self.relay.counts(),
        }
    }

    /// One `STATS` line for each session, in the order of their numbers:
    /// what its transport says of the path and has sent, and what its
    /// subgroup streams have handed to the transport, wait to write, and
    /// wait to open.
    fn log_stats(&self) {
        let mut sessions: Vec<_> = self.sessions.iter().collect();
        sessions.sort_unstable_by_key(|(session, _)| session.0);
        for (session, handle) in sessions {
            let (handed, writing) = handle.budget.traffic();
            let (bytes, objects, streams) = handle.budget.in_use();
            let sent = handle.connection.path().map_or(0, |path| path.sent_bytes);
            self.log(|| {
                format!(
                    "STATS session={}{} sent_bytes={sent} handed_bytes={handed} writing={writing} waiting_bytes={bytes} waiting_objects={objects} streams_not_done={streams} opening={}",
                    session.0,
                    path_now(&*handle.connection),
                    handle.opening.load(Ordering::Relaxed),
                )
            });
        }
    }

    fn log(&self, line: impl FnOnce() -> String) {
        if let Some(log) = &self.config.log {
            log.print(line());
        }
    }

    /// Notes which sessions have come to hold nothing, and starts their
    /// time; one that holds something again has no time running.
    fn watch_idle(&mut self) {
        let wait = self.config.idle_session_timeout;
        for (session, handle) in &mut self.sessions {
            let idle = handle.said_something && self.relay.holds_nothing(*session);
            match (handle.idle_until, idle) {
                (None, true) => {
                    let until = Instant::now() + wait;
                    handle.idle_until = Some(until);
                    self.idle.insert((until, *session));
                }
                (Some(until), false) => {
                    handle.idle_until = None;
                    self.idle.remove(&(until, *session));
                }
                _ => {}
            }
        }
    }

    /// The time the sessions in [`Self::idle`] may hold nothing is over for
    /// those due by now: they are closed.
    ///
    /// WHY `NO_ERROR`: §3.5 has no code for it ("`NO_ERROR` (0x0): The
    /// session is being terminated without an error"), and the peer did
    /// nothing wrong.
    fn idle_due(&mut self) {
        let now = Instant::now();
        let mut pending = VecDeque::new();
        while let Some(&(until, session)) = self.idle.first()
            && until <= now
        {
            self.idle.pop_first();
            self.close(session, NO_ERROR, "the session held nothing", &mut pending);
        }
        self.feed(pending);
    }

    /// The time a client has for its SETUP is over: a session that has
    /// said nothing on its control stream is closed.
    fn setup_due(&mut self, session: SessionKey) {
        let silent = self
            .sessions
            .get(&session)
            .is_some_and(|handle| !handle.said_something);
        if silent {
            let mut pending = VecDeque::new();
            // §3.5: "The session was closed because the peer took too long
            // to respond to a control message", as the clients of this
            // repository close one whose server sent no SETUP.
            self.close(
                session,
                TerminationCode::ControlMessageTimeout.code(),
                "no SETUP in time",
                &mut pending,
            );
            self.feed(pending);
        }
    }

    /// Starts the connection [`Action::ConnectParent`] asks for. The relay
    /// is told [`Event::ParentFailed`] at once where it has no parent.
    fn connect_parent(&mut self, uri: Vec<u8>, after: Duration, pending: &mut VecDeque<Event>) {
        let Some(parent) = &self.parent else {
            pending.push_back(Event::ParentFailed);
            return;
        };
        let Ok(uri) = String::from_utf8(uri) else {
            self.log(|| "PARENT_FAILED the New Session URI is not UTF-8".to_string());
            pending.push_back(Event::ParentFailed);
            return;
        };
        let uri = (!uri.is_empty()).then_some(uri);
        let (connect, events) = (Arc::clone(&parent.connect), self.events.clone());
        // WHY not among the tasks of a session: it is of none yet. It ends
        // by itself, within the time the connection has to be made.
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            let connected = connect(uri).await.map(Arc::new);
            // The hub is gone when this fails.
            let _ = events.send(Envelope::Parent(connected)).await;
        });
    }

    /// What came of [`Self::connect_parent`].
    fn parent_connected(&mut self, connected: Result<Arc<C>, String>) {
        match connected {
            Ok(connection) => {
                let session = SessionKey(self.next_session.fetch_add(1, Ordering::Relaxed));
                self.log(|| format!("PARENT_CONNECTED session={}", session.0));
                self.add_session(session, connection, Role::Client, Arrival::First);
            }
            Err(reason) => {
                self.log(|| format!("PARENT_FAILED reason={reason:?}"));
                self.feed(VecDeque::from([Event::ParentFailed]));
            }
        }
    }

    /// `role` is the relay's: [`Role::Client`] for its session with the
    /// parent relay, which is neither refused for nor counted in
    /// [`HubConfig::max_sessions`], since without it nobody of this relay
    /// gets the tracks it carries.
    fn add_session(
        &mut self,
        session: SessionKey,
        connection: Arc<C>,
        role: Role,
        arrival: Arrival,
    ) {
        let sending_away = self
            .sessions
            .values()
            .filter(|handle| handle.sent_away)
            .count();
        let served = self
            .sessions
            .values()
            .filter(|handle| handle.role == Role::Server && !handle.sent_away)
            .count();
        let full = role == Role::Server && served >= self.config.max_sessions;
        let send_to = match &self.config.when_full {
            Some(url) if full && arrival == Arrival::First && sending_away < MAX_SENDING_AWAY => {
                Some(sent_away_url(url))
            }
            _ => None,
        };
        if full && send_to.is_none() {
            self.log(|| session_refused_line(session));
            connection.close(TerminationCode::InternalError.code(), b"too many sessions");
            return;
        }
        self.log(|| session_opened_line(session));
        let stops = StreamStops::default();
        let (control, control_inbox) = mpsc::channel(self.config.session_queue_len);

        let uni = read_uni_streams(
            Arc::clone(&connection),
            self.config.streams.clone(),
            stops.clone(),
            self.events.clone(),
            move |event| uni_envelope(session, event),
        );
        let requests = read_request_streams(
            Arc::clone(&connection),
            self.config.streams.clone(),
            stops.clone(),
            self.events.clone(),
            {
                let log = self.config.log.clone();
                move |event| request_envelope(session, event, log.as_ref())
            },
        );
        let writer = tokio::spawn(write_control(
            session,
            Arc::clone(&connection),
            control_inbox,
            self.config.streams.trace.clone(),
            self.events.clone(),
        ));
        let watcher = tokio::spawn(watch_session(
            session,
            Arc::clone(&connection),
            [uni, requests],
            self.config.closed_read_timeout,
            self.config.log.clone(),
            self.events.clone(),
        ));
        // Among the tasks of the session: it goes with the session.
        let setup_due = tokio::spawn({
            let (events, wait) = (self.events.clone(), self.config.setup_timeout);
            async move {
                tokio::time::sleep(wait).await;
                // The hub is gone when this fails.
                let _ = events.send(Envelope::SetupDue { session }).await;
            }
        });
        self.sessions.insert(
            session,
            SessionHandle {
                connection,
                control,
                said_something: false,
                idle_until: None,
                requests: HashMap::new(),
                own_requests: HashMap::new(),
                stops,
                budget: SendBudget::within(self.config.send, &self.budget),
                subscriptions: HashMap::new(),
                opening: Arc::new(AtomicUsize::new(0)),
                tasks: vec![writer, watcher, setup_due],
                fetch_writes: Vec::new(),
                fetch_write_controls: HashMap::new(),
                sent_away: send_to.is_some(),
                role,
            },
        );
        let mut events = VecDeque::from([Event::SessionOpened { session, role }]);
        if let Some(url) = send_to {
            events.push_back(self.send_away(session, url));
        }
        self.feed(events);
    }

    /// Has the session that just opened sent to `url`
    /// ([`HubConfig::when_full`]): what the core is told, and the timer of
    /// its GOAWAY.
    fn send_away(&mut self, session: SessionKey, url: String) -> Event {
        self.log(|| session_sent_away_line(session, &url));
        let due = tokio::spawn({
            let events = self.events.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(SEND_AWAY_TIMEOUT_MS)).await;
                // The hub is gone when this fails.
                let _ = events.send(Envelope::SendAwayDue { session }).await;
            }
        });
        if let Some(handle) = self.sessions.get_mut(&session) {
            handle.tasks.push(due);
        }
        Event::SendAway {
            session,
            going_away: GoingAway {
                new_session_uri: url.into_bytes(),
                timeout_ms: SEND_AWAY_TIMEOUT_MS,
            },
        }
    }

    /// The session that was sent away is still there: §3.6, "The sender
    /// closes the session with a `GOAWAY_TIMEOUT` if the peer doesn't close
    /// the session within the indicated Timeout".
    fn send_away_due(&mut self, session: SessionKey) {
        let mut pending = VecDeque::new();
        self.close(session, GOAWAY_TIMEOUT, "sent away", &mut pending);
        self.feed(pending);
    }

    fn add_request(&mut self, session: SessionKey, stream: StreamKey, send: C::Send) {
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        let (commands, command_inbox) = mpsc::channel(self.config.session_queue_len);
        let (reset, reset_inbox) = oneshot::channel();
        let writer = tokio::spawn(write_request(
            session,
            stream,
            send,
            (command_inbox, reset_inbox),
            self.config.streams.trace.clone(),
            self.config.log.clone(),
            self.events.clone(),
        ));
        handle
            .requests
            .insert(stream, RequestWriter { commands, reset });
        // Finished writers are dropped from the list as new ones come, so that
        // it holds about as many as there are open requests.
        handle.tasks.retain(|task| !task.is_finished());
        handle.tasks.push(writer);
    }

    /// Feeds events to the relay until nothing is left to do. Carrying out an
    /// action can produce another event, such as a budget that turned out
    /// used up.
    fn feed(&mut self, mut pending: VecDeque<Event>) {
        // WHY not after an Object or a stream of Objects: those come by
        // the thousand, and change nothing of what a session holds.
        let mut may_have_changed = false;
        while let Some(event) = pending.pop_front() {
            may_have_changed |= !matches!(
                event,
                Event::Object { .. }
                    | Event::ObjectBegun { .. }
                    | Event::DataStreamOpened { .. }
                    | Event::OutStreamOpened { .. }
                    | Event::OutStreamStopped { .. }
                    | Event::OutStreamClosed { .. }
                    | Event::FetchStreamWritten { .. }
            );
            let closed = match &event {
                Event::SessionClosed { session } => Some(*session),
                _ => None,
            };
            // What a publisher answered to a request of the relay: the one
            // thing the relay acts on that none of its own lines shows.
            if let Event::OwnRequestMessage {
                session,
                request,
                message,
            } = &event
            {
                self.log(|| recv_line(*session, *request, message));
            }
            if let Event::DrainTimeout { session, wait, .. } = &event {
                self.drain_ended(*session, *wait, "timeout");
            }
            if let Event::FetchPipeEnded {
                session,
                stream,
                how,
            } = &event
            {
                // The task is done; nothing is left to cut short. WHY an end
                // for a FETCH that is not passed on any more is dropped: the
                // relay ordered the passing on stopped (`AbortFetchPipe`)
                // while the task was ending by itself, and is told of no end
                // after that order (`Event::FetchPipeEnded`).
                if self.fetch_pipes.remove(&(*session, *stream)).is_none() {
                    continue;
                }
                self.log(|| fetch_pipe_ended_line(*session, *stream, *how));
            }
            // The peer's half of the stream is gone: nothing is left to wait
            // for there.
            if let Event::RequestClosed {
                session, stream, ..
            } = &event
            {
                self.timers.stop_request_close(*session, *stream);
                self.timers.stop_rendezvous(*session, *stream);
            }
            let actions = self.relay.handle(event);
            for action in actions {
                self.execute(action, &mut pending);
            }
            if let Some(session) = closed {
                self.forget_session(session);
            }
        }
        if may_have_changed {
            self.watch_idle();
        }
        // A session goes through here when it is closed, by the relay or by
        // the peer.
        if self.drained.is_some()
            && self
                .sessions
                .values()
                .all(|handle| handle.role == Role::Client)
        {
            self.parent_drained();
        }
    }

    /// The clients of the relay that goes away are gone: the sessions with
    /// the parent that are left are closed, and the relay is drained.
    ///
    /// WHY not waited for: the parent was told the relay goes (§10.4, a
    /// client's GOAWAY "intends to close the session soon") and has no
    /// reason to close; it would be closed with `GOAWAY_TIMEOUT` (§3.5:
    /// "The session was closed because the peer took too long to close the
    /// session in response to a GOAWAY") when the Timeout is over, and the
    /// relay would stop that much later. WHY `NO_ERROR`: neither side did
    /// anything wrong. WHY kept until now: the clients that were still
    /// there got their tracks through it.
    fn parent_drained(&mut self) {
        let Some(drained) = self.drained.take() else {
            return;
        };
        let parent_sessions: Vec<SessionKey> = self.sessions.keys().copied().collect();
        let mut pending = VecDeque::new();
        for session in parent_sessions {
            self.close(session, NO_ERROR, "the relay is stopping", &mut pending);
        }
        self.feed(pending);
        let _ = drained.send(());
    }

    /// Finishes the relay's direction of a request stream of a peer, and
    /// gives the peer its time to end its own.
    fn finish_request(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        action: &Action,
        pending: &mut VecDeque<Event>,
    ) {
        self.request(session, stream, RequestCommand::Finish, pending);
        if let Some(handle) = self.sessions.get_mut(&session) {
            handle.requests.remove(&stream);
        }
        // Nothing to wait for where the peer ended its direction before:
        // the relay has let go of the stream then.
        if self.relay.has_request_stream(session, stream) {
            self.set_timer(action, pending);
        }
    }

    fn log_action(&self, action: &Action) {
        if let Some(line) = describe(action) {
            self.log(|| line);
        }
    }

    /// Queues an Object for a subgroup stream to a subscriber.
    fn send_object(
        &mut self,
        out_stream: OutStreamKey,
        object: Arc<SubgroupObject>,
        pending: &mut VecDeque<Event>,
    ) {
        let full = self
            .out_streams
            .get(&out_stream)
            .and_then(|(session, writer)| writer.send(object).err().map(|full| (*session, full)));
        if let Some((session, full)) = full {
            pending.push_back(self.queue_full(out_stream, session, full));
        }
        self.keep_to_the_relay_s_bound(out_stream, pending);
    }

    /// Where more waits to be written to all the subscribers than
    /// [`HubConfig::send_for_the_relay_bytes`], the core is told so
    /// (`SendLimit::Relay`) with the bytes waiting for each subscription that
    /// has some, named by its stream opened last, and chooses the one that
    /// ends.
    ///
    /// WHY by subscription and not by stream: this runs for every Object
    /// while the relay is over, which is when it has the most to do, and a
    /// relay has far fewer subscriptions than streams. WHY the stream opened
    /// last names it: a subscription with bytes waiting has a stream not
    /// done, the latest the likeliest; where that one is gone the core
    /// passes the subscription over, and the next Object tells it again.
    fn keep_to_the_relay_s_bound(&self, out_stream: OutStreamKey, pending: &mut VecDeque<Event>) {
        if self.budget.in_use().0 <= self.config.send_for_the_relay_bytes {
            return;
        }
        let held = self
            .sessions
            .values()
            .flat_map(|handle| handle.subscriptions.values())
            .map(|subscription| (subscription.latest, subscription.budget.in_use().0))
            .filter(|(_, bytes)| *bytes > 0)
            .collect();
        pending.push_back(Event::SendQueueFull {
            out_stream,
            limit: SendLimit::Relay,
            held,
        });
    }

    /// What the core is told where the budget `full` names of a stream to
    /// `session` is used up: for the subscription's own, nothing more; for
    /// the session's, what each stream not done to `session` has carried
    /// ([`OutgoingSubgroup::carried`]).
    fn queue_full(&self, out_stream: OutStreamKey, session: SessionKey, full: QueueFull) -> Event {
        if full.level == SUBSCRIPTION_BUDGET {
            return Event::SendQueueFull {
                out_stream,
                limit: SendLimit::Subscription,
                held: Vec::new(),
            };
        }
        let held = self
            .out_streams
            .iter()
            .filter(|(_, (of, _))| *of == session)
            .map(|(key, (_, writer))| (*key, writer.carried()))
            .collect();
        Event::SendQueueFull {
            out_stream,
            limit: SendLimit::Session,
            held,
        }
    }

    /// Hands a control message to the writer of the session's control
    /// stream.
    fn send_control(
        &mut self,
        session: SessionKey,
        message: ControlMessage,
        pending: &mut VecDeque<Event>,
    ) {
        let Some(handle) = self.sessions.get(&session) else {
            return;
        };
        // Full: the session does not take its control messages.
        // Closed: its control stream could not be written, which the
        // writer has reported or is about to.
        if handle.control.try_send(message).is_err() {
            self.close(
                session,
                TerminationCode::InternalError.code(),
                "control messages cannot be sent",
                pending,
            );
        }
    }

    fn execute(&mut self, action: Action, pending: &mut VecDeque<Event>) {
        self.log_action(&action);
        let Some(action) = self.execute_upstream(action, pending) else {
            return;
        };
        match action {
            Action::SendControl { session, message } => {
                self.send_control(session, message, pending);
            }
            Action::SendRequest {
                session,
                stream,
                message,
            } => {
                // An answer, whatever it is: the SUBSCRIBE is held no longer.
                self.timers.stop_rendezvous(session, stream);
                self.request(session, stream, RequestCommand::Send(message), pending);
            }
            Action::FinishRequest { session, stream } => {
                self.finish_request(session, stream, &action, pending);
            }
            Action::ResetRequest {
                session,
                stream,
                code,
            } => self.reset_request(session, stream, code),
            opening @ Action::OpenDataStream { .. } => self.open_data_stream(&opening, pending),
            Action::SetDeliveryTimeout {
                out_stream,
                subgroup_delivery_timeout: asked,
            } => self.set_delivery_timeout(out_stream, asked),
            Action::SendObject { out_stream, object } => {
                self.send_object(out_stream, object, pending);
            }
            // The writer stays in the table until it reports that it is done,
            // so that it can still be reset.
            Action::FinishDataStream { out_stream } => {
                if let Some((_, writer)) = self.out_streams.get(&out_stream) {
                    writer.finish();
                }
            }
            Action::ResetDataStream { out_stream, code } => {
                if let Some((_, writer)) = self.out_streams.remove(&out_stream) {
                    writer.reset(code);
                }
            }
            // Both stop a stream of the peer: its readers go by one numbering.
            Action::StopDataStream {
                session,
                stream,
                code,
            }
            | Action::StopRequest {
                session,
                stream,
                code,
            } => {
                if let Some(handle) = self.sessions.get(&session) {
                    handle.stops.stop(stream.0, code);
                }
            }
            Action::StartDrainTimer { session, wait, .. } => {
                self.drains.insert((session, wait), Instant::now());
                self.set_timer(&action, pending);
            }
            Action::StopDrainTimer { .. }
            | Action::StopEarlyTimer { .. }
            | Action::StopJoiningFetchTimer { .. }
            | Action::StartEarlyTimer { .. }
            | Action::StartCheckTimer { .. }
            | Action::StartJoiningFetchTimer { .. }
            | Action::StartReceiptTimer { .. }
            | Action::StopReceiptTimer { .. }
            | Action::StartRendezvousTimer { .. }
            | Action::StartAnnouncementTimer { .. }
            | Action::StartParentAnnouncementTimer { .. }
            | Action::StartFetchEndTimer { .. }
            | Action::StartUpstreamTimer { .. }
            | Action::StartFetchStreamTimer { .. }
            | Action::StopUpstreamTimer { .. } => self.set_timer(&action, pending),
            Action::CloseSession {
                session,
                code,
                reason,
            } => self.close(session, code, &reason, pending),
            Action::ConnectParent { uri, after_ms } => {
                self.connect_parent(uri, Duration::from_millis(after_ms), pending);
            }
            // Taken by `execute_upstream` above.
            Action::OpenRequest { .. }
            | Action::SendOwnRequest { .. }
            | Action::FinishOwnRequest { .. }
            | Action::CancelOwnRequest { .. }
            | Action::PipeFetch { .. }
            | Action::AbortFetchPipe { .. }
            | Action::WriteFetchStream { .. }
            | Action::AbortFetchWrite { .. }
            | Action::SetFetchSendOrder { .. }
            | Action::NoteHeldBeyondCache { .. }
            // For the log alone (`describe`).
            | Action::SubscribeHeld { .. } => {}
        }
    }

    /// Carries out the actions that are about a request the relay opened, or
    /// about a fetch stream. Returns every other action.
    fn execute_upstream(
        &mut self,
        action: Action,
        pending: &mut VecDeque<Event>,
    ) -> Option<Action> {
        match action {
            Action::OpenRequest {
                session,
                request,
                message,
            } => self.open_request(session, request, message),
            Action::SendOwnRequest {
                session,
                request,
                message,
            } => self.send_own_request(session, request, message),
            Action::FinishOwnRequest { session, request } => {
                self.own_request(session, request, OwnRequestEnding::Finish);
            }
            Action::CancelOwnRequest {
                session,
                request,
                code,
            } => self.own_request(session, request, OwnRequestEnding::Cancel(code)),
            Action::PipeFetch {
                from,
                from_stream,
                session,
                stream,
                request_id,
                upstream_wait_ms,
                downstream_wait_ms,
                send_order,
            } => {
                let waits = PipeWaits {
                    upstream: Duration::from_millis(upstream_wait_ms),
                    downstream: Duration::from_millis(downstream_wait_ms),
                };
                let header = FetchHeader { request_id };
                self.pipe_fetch(
                    (from, from_stream),
                    (session, stream),
                    (header, send_order),
                    waits,
                    pending,
                );
            }
            Action::AbortFetchPipe {
                session,
                stream,
                code,
            } => {
                if let Some(control) = self.fetch_pipes.remove(&(session, stream)) {
                    // The task may have just ended; then nobody needs this.
                    let _ = control.abort.send(code);
                }
            }
            Action::WriteFetchStream {
                session,
                stream,
                request_id,
                pieces,
                wait_ms,
                send_order,
            } => {
                let stream_of = (request_id, send_order);
                self.write_fetch_stream((session, stream), stream_of, pieces, wait_ms);
            }
            Action::AbortFetchWrite {
                session,
                stream,
                code,
            } => {
                let control = self
                    .sessions
                    .get_mut(&session)
                    .and_then(|handle| handle.fetch_write_controls.remove(&stream));
                if let Some(control) = control {
                    // The task may have just ended; then nobody needs this.
                    let _ = control.abort.send(code);
                }
            }
            Action::SetFetchSendOrder {
                session,
                stream,
                send_order,
            } => {
                let written = self
                    .sessions
                    .get(&session)
                    .and_then(|handle| handle.fetch_write_controls.get(&stream));
                let control = self.fetch_pipes.get(&(session, stream)).or(written);
                if let Some(control) = control {
                    // Nobody reads it when the task has ended.
                    control.send_order.send_replace(send_order);
                }
            }
            other => return Some(other),
        }
        None
    }

    /// Writes a whole fetch stream that the relay made itself, and tells the
    /// relay when that is over.
    fn write_fetch_stream(
        &mut self,
        (session, stream): (SessionKey, StreamKey),
        (request_id, send_order): (u64, i32),
        pieces: Vec<FetchPiece>,
        wait_ms: u64,
    ) {
        let written = move |given_up| Event::FetchStreamWritten {
            session,
            stream,
            given_up,
        };
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        // The relay lets a session have only so many of these at once, so
        // the lists stay short. The tasks go with the session.
        handle.fetch_writes.retain(|task| !task.is_finished());
        handle
            .fetch_write_controls
            .retain(|_, control| !control.abort.is_closed());
        let (control, aborted, reordered) = FetchControl::new(send_order);
        handle.fetch_write_controls.insert(stream, control);
        let pieces_bytes: usize = pieces
            .iter()
            .map(|piece| match piece {
                FetchPiece::Object(cached) => xmoq_relay_core::object_size(&cached.object),
                FetchPiece::EndOfRange(..) => 0,
            })
            .sum();
        let writing = FetchWrite {
            connection: Arc::clone(&handle.connection),
            header: FetchHeader { request_id },
            send_order: reordered,
            pieces,
            patience: Duration::from_millis(wait_ms),
            whole: self.config.cached_fetch_whole(pieces_bytes),
            trace: self.config.streams.trace.clone(),
            label: format!("session#{} fetch#{request_id}", session.0),
        };
        let events = self.events.clone();
        let log = self.config.log.clone();
        handle.fetch_writes.push(tokio::spawn(async move {
            // Told whatever became of the write. A panic: the writer that
            // went with it reset the stream.
            let ended = in_a_task_that_ends_with_this_call(writing.run(aborted))
                .await
                .unwrap_or(Written {
                    how: WriteEnd::Failed,
                    pieces: 0,
                });
            if let Some(log) = &log {
                log.print(ended.line(session, stream));
            }
            // The hub is gone when this fails, and the relay with it.
            let _ = events.send(Envelope::Core(written(ended.given_up()))).await;
        }));
    }

    /// Starts passing a publisher's fetch stream on to a subscriber.
    fn pipe_fetch(
        &mut self,
        (from, from_stream): (SessionKey, StreamKey),
        subscriber: (SessionKey, StreamKey),
        (header, send_order): (FetchHeader, i32),
        waits: PipeWaits,
        pending: &mut VecDeque<Event>,
    ) {
        let (session, stream) = subscriber;
        let ended = move |how| Event::FetchPipeEnded {
            session,
            stream,
            how,
        };
        // Before anything can end it: an end that is reported for a FETCH
        // without this entry is one the relay does not wait for (`feed`).
        let (control, aborted, reordered) = FetchControl::new(send_order);
        self.fetch_pipes.insert(subscriber, control);
        let Some(publisher) = self.sessions.get(&from) else {
            pending.push_back(ended(FetchPipeEnd::UpstreamLost));
            return;
        };
        // Taken at once, so that the publisher's stream frees its slot among
        // the streams read of that session.
        let Some(body) = publisher.stops.take_fetch_body(from_stream.0) else {
            pending.push_back(ended(FetchPipeEnd::UpstreamLost));
            return;
        };
        // The body, dropped here, stops the publisher's stream.
        let Some(handle) = self.sessions.get(&session) else {
            pending.push_back(ended(FetchPipeEnd::DownstreamLost));
            return;
        };
        let pipe = FetchPipe {
            connection: Arc::clone(&handle.connection),
            header,
            body,
            waits,
            reordered,
            trace: self.config.streams.trace.clone(),
            label: format!("session#{} fetch#{}", session.0, header.request_id),
        };
        let events = self.events.clone();
        tokio::spawn(async move {
            // WHY a task of its own for the passing on: the relay is told of
            // its end here whatever becomes of it, a panic included, or the
            // FETCH would stay in the relay for as long as its sessions do.
            // Both streams go with that task: the writer resets the
            // subscriber's, and the body, dropped, stops the publisher's.
            let how = match in_a_task_that_ends_with_this_call(pipe.run(aborted)).await {
                Ok(how) => how,
                Err(_) => Some(FetchPipeEnd::UpstreamLost),
            };
            if let Some(how) = how {
                let _ = events.send(Envelope::Core(ended(how))).await;
            }
        });
    }

    /// The subscriber changed its `SUBGROUP_DELIVERY_TIMEOUT`
    /// ([`OutgoingSubgroup::set_delivery_timeout`]).
    fn set_delivery_timeout(&self, out_stream: OutStreamKey, asked: Option<u64>) {
        if let Some((_, writer)) = self.out_streams.get(&out_stream) {
            writer.set_delivery_timeout(asked.map(Duration::from_millis));
        }
    }

    /// The budget of the subscription of `session` the relay gave
    /// `track_alias`, made where it has none.
    fn subscription_budget(
        &mut self,
        session: SessionKey,
        track_alias: u64,
        out_stream: OutStreamKey,
    ) -> Option<Arc<SendBudget>> {
        let limits = SendLimits {
            delivery_patience: self.config.send.delivery_patience,
            ..self.config.send_for_one_subscription
        };
        let handle = self.sessions.get_mut(&session)?;
        // WHY dropped here: a budget no writer holds holds nothing, and the
        // subscriptions of a session come and go.
        handle
            .subscriptions
            .retain(|_, subscription| Arc::strong_count(&subscription.budget) > 1);
        let subscription =
            handle
                .subscriptions
                .entry(track_alias)
                .or_insert_with(|| SubscriptionBudget {
                    budget: SendBudget::within(limits, &handle.budget),
                    latest: out_stream,
                });
        subscription.latest = out_stream;
        Some(Arc::clone(&subscription.budget))
    }

    /// Starts the writer of a subgroup stream towards a subscriber, which
    /// carries out [`Action::OpenDataStream`]; another action is none of its
    /// business. `asked` is the `SUBGROUP_DELIVERY_TIMEOUT` of the
    /// subscription, in milliseconds, where its SUBSCRIBE had one.
    fn open_data_stream(&mut self, opening: &Action, pending: &mut VecDeque<Event>) {
        let &Action::OpenDataStream {
            session,
            out_stream,
            header,
            subgroup_delivery_timeout: asked,
            send_order,
        } = opening
        else {
            return;
        };
        let delivery_timeout = asked.map(Duration::from_millis);
        let Some(subscription) = self.subscription_budget(session, header.track_alias, out_stream)
        else {
            return;
        };
        let Some(handle) = self.sessions.get(&session) else {
            return;
        };
        let events = self.events.clone();
        let log = self.config.log.clone();
        // For the log: when the stream was asked for, how many of the
        // session wait to be opened, and how the path stands.
        let asked = Instant::now();
        let slow_open = self.config.slow_open;
        let opening = Arc::clone(&handle.opening);
        opening.fetch_add(1, Ordering::Relaxed);
        let waits = Arc::new(AtomicBool::new(true));
        let connection = Arc::clone(&handle.connection);
        let report = move |event| {
            let events = events.clone();
            let log = log.clone();
            // Opened, or over without that: it waits to be opened no more.
            let waited = waits.swap(false, Ordering::Relaxed);
            let still_opening = if waited {
                opening.fetch_sub(1, Ordering::Relaxed).saturating_sub(1)
            } else {
                opening.load(Ordering::Relaxed)
            };
            let after = asked.elapsed();
            if matches!(event, OutgoingEvent::Opened)
                && after >= slow_open
                && let Some(log) = &log
            {
                log.print(format!(
                    "DATA_OPENED session={} out={} after_ms={} opening={still_opening}{}",
                    session.0,
                    out_stream.0,
                    after.as_millis(),
                    path_now(&*connection),
                ));
            }
            async move {
                let event = match event {
                    OutgoingEvent::Opened => Event::OutStreamOpened { out_stream },
                    OutgoingEvent::Closed => Event::OutStreamClosed { out_stream },
                    // For the log only: the writer resets the stream and
                    // reports it closed.
                    OutgoingEvent::TimedOut => {
                        if let Some(log) = &log {
                            log.print(format!(
                                "DELIVERY_TIMED_OUT session={} out={}",
                                session.0, out_stream.0
                            ));
                        }
                        return;
                    }
                    OutgoingEvent::Stopped(code) => {
                        if let Some(log) = &log {
                            log.print(format!(
                                "STOP_SENDING_RECEIVED session={} out={} {}",
                                session.0,
                                out_stream.0,
                                stop_code(code)
                            ));
                        }
                        Event::OutStreamStopped { out_stream }
                    }
                };
                let _ = events.send(Envelope::Core(event)).await;
            }
        };
        let writer = OutgoingSubgroup::spawn(
            Arc::clone(&handle.connection),
            header,
            (delivery_timeout, send_order),
            &subscription,
            report,
            self.config.streams.trace.clone(),
            format!("session#{} out#{}", session.0, out_stream.0),
        );
        match writer {
            Ok(writer) => {
                self.out_streams.insert(out_stream, (session, writer));
            }
            Err(full) => {
                // Too many streams of this subscription, of this subscriber
                // or of the relay are not written out. No writer was
                // started, so the hub says what one would have said.
                pending.push_back(self.queue_full(out_stream, session, full));
                pending.push_back(Event::OutStreamClosed { out_stream });
            }
        }
    }

    /// Resets a request stream of the peer in both directions, and forgets
    /// it.
    fn reset_request(&mut self, session: SessionKey, stream: StreamKey, code: u32) {
        if let Some(handle) = self.sessions.get_mut(&session) {
            if let Some(writer) = handle.requests.remove(&stream) {
                // An error means the writer is gone, with the stream.
                let _ = writer.reset.send(code);
            }
            handle.stops.stop(stream.0, code);
        }
    }

    /// Stops the timer an action asks to stop. Says whether it was such an
    /// action.
    fn stop_timer(&mut self, action: &Action) -> bool {
        match *action {
            Action::StopUpstreamTimer { session, request } => {
                self.timers.stop_upstream(session, request);
            }
            Action::StopDrainTimer { session, wait } => {
                self.timers.stop_drain(session, wait);
                self.drain_ended(session, wait, "stop");
            }
            Action::StopEarlyTimer { session, stream } => self.timers.stop_early(session, stream),
            Action::StopJoiningFetchTimer { session, stream } => {
                self.timers.stop_joining(session, stream);
            }
            Action::StopReceiptTimer { session, stream } => {
                self.timers.stop_receipt(session, stream);
            }
            _ => return false,
        }
        true
    }

    /// Starts or stops the timer an action asks for.
    fn set_timer(&mut self, action: &Action, pending: &mut VecDeque<Event>) {
        if self.stop_timer(action) {
            return;
        }
        let (session, wait, event) = match *action {
            Action::StartDrainTimer {
                session,
                track_alias,
                wait,
            } => (
                session,
                self.config.drain_timeout,
                Event::DrainTimeout {
                    session,
                    track_alias,
                    wait,
                },
            ),
            Action::StartEarlyTimer { session, stream } => (
                session,
                self.config.early_stream_timeout,
                Event::EarlyStreamTimeout { session, stream },
            ),
            Action::StartCheckTimer {
                session,
                stream,
                wait_ms,
            } => (
                session,
                Duration::from_millis(wait_ms),
                Event::CheckStreamTimeout { session, stream },
            ),
            // The time the peer gets to end its half of a request stream
            // the relay finished its own half of.
            Action::FinishRequest { session, stream } => (
                session,
                self.config.request_close_timeout,
                Event::RequestTimeout { session, stream },
            ),
            // WHY the time a stream gets for its first bytes: what is waited
            // for is the first message of a stream the peer has opened, or
            // is about to.
            Action::StartJoiningFetchTimer { session, stream } => (
                session,
                self.config.streams.first_bytes_timeout,
                Event::JoiningFetchTimeout { session, stream },
            ),
            Action::StartReceiptTimer {
                session,
                stream,
                wait_ms,
            } => (
                session,
                Duration::from_millis(wait_ms),
                Event::ReceiptTimeout { session, stream },
            ),
            Action::StartAnnouncementTimer { session, wait_ms } => (
                session,
                Duration::from_millis(wait_ms),
                Event::AnnouncementTimeout { session },
            ),
            Action::StartParentAnnouncementTimer { session, wait_ms } => (
                session,
                Duration::from_millis(wait_ms),
                Event::ParentAnnouncementTimeout { session },
            ),
            Action::StartRendezvousTimer {
                session,
                stream,
                wait_ms,
            } => (
                session,
                Duration::from_millis(wait_ms),
                Event::RendezvousTimeout { session, stream },
            ),
            Action::StartFetchEndTimer {
                session,
                stream,
                wait_ms,
            } => (
                session,
                Duration::from_millis(wait_ms),
                Event::FetchEndTimeout { session, stream },
            ),
            Action::StartUpstreamTimer { session, request } => (
                session,
                self.config.upstream_timeout,
                Event::UpstreamTimeout { session, request },
            ),
            Action::StartFetchStreamTimer {
                session,
                request,
                wait_ms,
            } => (
                session,
                Duration::from_millis(wait_ms),
                Event::UpstreamTimeout { session, request },
            ),
            _ => return,
        };
        let limit = timer_limit(&event, self.config.max_timers_per_session);
        if let Some(event) = self.timers.start(session, limit, wait, event) {
            // WHY at once: see `Action::StartUpstreamTimer`. Whatever the
            // timer guarded is let go of now instead of never.
            self.log(|| format!("TIMER_NOT_STARTED session={} too many timers", session.0));
            pending.push_back(event);
        }
    }

    /// Starts the task of a request the relay opens on a session.
    fn open_request(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        message: ControlMessage,
    ) {
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        // Room for the updates the session lets wait for their answer.
        let room = usize::try_from(MAX_UNANSWERED_UPDATES).unwrap_or(usize::MAX);
        let (updates, updates_inbox) = mpsc::channel(room);
        let (ending, ending_inbox) = oneshot::channel();
        let task = tokio::spawn(run_own_request(
            session,
            request,
            Arc::clone(&handle.connection),
            message,
            (updates_inbox, ending_inbox),
            self.config.streams.trace.clone(),
            self.events.clone(),
        ));
        handle
            .own_requests
            .insert(request, OwnRequestTask { updates, ending });
        handle.tasks.retain(|task| !task.is_finished());
        handle.tasks.push(task);
    }

    /// Hands one more message to the task of a request the relay opened.
    fn send_own_request(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        message: ControlMessage,
    ) {
        let Some(task) = self
            .sessions
            .get(&session)
            .and_then(|handle| handle.own_requests.get(&request))
        else {
            return;
        };
        // Full: the task is stuck writing what came before, and the session
        // lets no more updates wait than there is room for. Closed: the task
        // ended by itself, with the stream.
        if task.updates.try_send(message).is_err() {
            self.log(|| {
                format!(
                    "OWN_REQUEST_NOT_SENT session={} own={}",
                    session.0, request.0
                )
            });
        }
    }

    /// Ends the task of a request the relay opened, and the timer for its
    /// answer with it.
    fn own_request(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        ending: OwnRequestEnding,
    ) {
        self.timers.stop_upstream(session, request);
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        if let Some(task) = handle.own_requests.remove(&request) {
            // An error: the task ended by itself, with the stream.
            let _ = task.ending.send(ending);
        }
    }

    /// Queues a command for the writer of a request stream. A stream whose
    /// queue is full belongs to a session that does not take its messages, and
    /// the session is closed; but for the stream of a `SUBSCRIBE_NAMESPACE`,
    /// which is reset with `EXCESSIVE_LOAD` and ended alone.
    ///
    /// WHY the stream alone there: §10.18 has "If the publisher is unable to
    /// send NAMESPACE or `NAMESPACE_DONE` messages in a timely manner because
    /// the `SUBSCRIBE_NAMESPACE` response stream is blocked by flow control,
    /// the publisher MAY reset the `SUBSCRIBE_NAMESPACE` response stream", and
    /// the subscriber takes the reset as a `NAMESPACE_DONE` for each. Nothing
    /// else of the session waits for that stream.
    fn request(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        command: RequestCommand,
        pending: &mut VecDeque<Event>,
    ) {
        let Some(writer) = self
            .sessions
            .get(&session)
            .and_then(|handle| handle.requests.get(&stream))
        else {
            return;
        };
        let of_a_namespace_subscription = matches!(
            command,
            RequestCommand::Send(ControlMessage::Namespace(_) | ControlMessage::NamespaceDone(_))
        );
        // Closed means the writer ended because the peer stopped the stream,
        // which it reports itself.
        if let Err(mpsc::error::TrySendError::Full(_)) = writer.commands.try_send(command) {
            if of_a_namespace_subscription {
                self.reset_full_request(session, stream, pending);
                return;
            }
            self.close(
                session,
                TerminationCode::InternalError.code(),
                "too many messages are waiting to be sent",
                pending,
            );
        }
    }

    /// Resets a request stream whose queue is full, ahead of what waits in it,
    /// stops the peer's direction, and tells the core that the request is
    /// over.
    fn reset_full_request(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        pending: &mut VecDeque<Event>,
    ) {
        let code = stream_reset_code::EXCESSIVE_LOAD;
        let Some(handle) = self.sessions.get_mut(&session) else {
            return;
        };
        if let Some(writer) = handle.requests.remove(&stream) {
            // An error means the writer is gone, with the stream.
            let _ = writer.reset.send(code);
        }
        handle.stops.stop(stream.0, code);
        if let Some(log) = &self.config.log {
            log.print(format!(
                "RESET_FULL_REQUEST session={} stream={} code={code:#x}",
                session.0, stream.0
            ));
        }
        pending.push_back(Event::RequestClosed {
            session,
            stream,
            how: RequestEnd::Aborted,
        });
    }

    /// Closes every session, and answers when the peers have been told or
    /// [`CLOSE_REACHES_THE_PEER`] later ([`HubHandle::stop_with`],
    /// [`HubHandle::go_away`]).
    fn stop(&mut self, stopping: Stopping) {
        let (answer, code, reason) = match stopping {
            Stopping::GoAway {
                going_away,
                drained,
            } => {
                self.log(|| going_away_line(self.sessions.len(), &going_away));
                self.drained = Some(drained);
                self.feed(VecDeque::from([Event::GoingAway(going_away)]));
                return;
            }
            Stopping::Close {
                answer,
                code,
                reason,
            } => (answer, code, reason),
        };
        let sessions: Vec<SessionKey> = self.sessions.keys().copied().collect();
        let connections: Vec<Arc<C>> = self
            .sessions
            .values()
            .map(|handle| Arc::clone(&handle.connection))
            .collect();
        self.log(|| stopping_line(sessions.len()));
        let mut pending = VecDeque::new();
        for session in sessions {
            self.close(session, code, reason, &mut pending);
        }
        self.feed(pending);
        tokio::spawn(async move {
            let all_closed = async {
                for connection in &connections {
                    connection.closed().await;
                }
            };
            let _ = tokio::time::timeout(CLOSE_REACHES_THE_PEER, all_closed).await;
            let _ = answer.send(());
        });
    }

    /// Closes a session and tells the relay it is gone.
    fn close(
        &mut self,
        session: SessionKey,
        code: u32,
        reason: &str,
        pending: &mut VecDeque<Event>,
    ) {
        let Some(handle) = self.sessions.remove(&session) else {
            return;
        };
        if let Some(until) = handle.idle_until {
            self.idle.remove(&(until, session));
        }
        self.log(|| close_session_line(session, code, reason));
        // WHY here and not through a writer of the session: the session is
        // forgotten right after, which stops its writers. A close left in a
        // queue would be lost, and the peer would see the connection go away
        // without the code.
        handle.connection.close(code, reason.as_bytes());
        // WHY the streams of the session are kept as they are until the
        // connection is closed, and not let go of with the session: a stream
        // that is dropped is reset or stopped, each of which the peer is
        // told at once, on its own, while the code of the close travels in
        // a capsule on another stream. A peer would then learn of a control
        // stream that was reset before it learns why, or instead of it, in
        // whichever order the frames came. The draft has nothing on the
        // streams of a session that is terminated (§3.5).
        let of_the_session: Vec<OutStreamKey> = self
            .out_streams
            .iter()
            .filter(|(_, (owner, _))| *owner == session)
            .map(|(out_stream, _)| *out_stream)
            .collect();
        let writers: Vec<OutgoingSubgroup> = of_the_session
            .iter()
            .filter_map(|out_stream| self.out_streams.remove(out_stream))
            .map(|(_, writer)| writer)
            .collect();
        let connection = Arc::clone(&handle.connection);
        self.closing.insert(session);
        tokio::spawn(async move {
            let _ = tokio::time::timeout(CLOSE_REACHES_THE_PEER, connection.closed()).await;
            drop((handle, writers));
        });
        pending.push_back(Event::SessionClosed { session });
    }

    fn forget_session(&mut self, session: SessionKey) {
        let idle_until = self
            .sessions
            .remove(&session)
            .map(|handle| handle.idle_until);
        if let Some(Some(until)) = idle_until {
            self.idle.remove(&(until, session));
        }
        if idle_until.is_some() | self.closing.remove(&session) {
            self.log(|| session_closed_line(session));
        }
        // Every writer of the session, also one that was finished and has not
        // written everything out.
        self.out_streams.retain(|_, (owner, writer)| {
            if *owner == session {
                writer.reset(stream_reset_code::SESSION_CLOSED);
            }
            *owner != session
        });
        self.timers.forget_session(session);
        self.drains.retain(|(owner, _), _| *owner != session);
    }

    /// Says that a wait for the last streams of a source is over, and how
    /// long it ran by the relay's clock: `by` is "timeout" where it ran out,
    /// and "stop" where the source was over before it.
    fn drain_ended(&mut self, session: SessionKey, wait: u64, by: &str) {
        let Some(began) = self.drains.remove(&(session, wait)) else {
            return;
        };
        let after_ms = began.elapsed().as_millis();
        self.log(|| drain_ended_line(session, after_ms, by));
    }
}

// The lines of the sessions and of the relay as a whole. The interop scenes
// wait for each (the scripts are named on each function), and the tests in
// `line_tests` pin them.

/// `interop/when-full.ts`.
fn session_refused_line(session: SessionKey) -> String {
    format!("SESSION_REFUSED session={} too many sessions", session.0)
}

/// `interop/disconnect.ts`, `interop/no-answer.ts`, `interop/rendezvous.ts`.
fn session_opened_line(session: SessionKey) -> String {
    format!("SESSION_OPENED session={}", session.0)
}

/// `interop/when-full.ts`.
fn session_sent_away_line(session: SessionKey, url: &str) -> String {
    format!("SESSION_SENT_AWAY session={} uri={url}", session.0)
}

/// `interop/fetch.ts`, `interop/rendezvous.ts`, `interop/disconnect.ts`.
fn recv_line(session: SessionKey, request: OwnRequestKey, message: &ControlMessage) -> String {
    format!(
        "RECV session={} own={} {}",
        session.0,
        request.0,
        name(message)
    )
}

/// `interop/quiet-peer.ts`, `interop/video.ts`, `interop/watch-rust.ts`.
fn going_away_line(sessions: usize, going_away: &GoingAway) -> String {
    format!(
        "GOING_AWAY sessions={} timeout_ms={} uri={}",
        sessions,
        going_away.timeout_ms,
        String::from_utf8_lossy(&going_away.new_session_uri)
    )
}

/// `interop/watch-rust.ts`.
fn stopping_line(sessions: usize) -> String {
    format!("STOPPING sessions={sessions}")
}

/// `interop/auth-edge.ts`, `interop/disconnect.ts`, `interop/new-group.ts`,
/// `interop/no-answer.ts`, `interop/quiet-peer.ts`, `interop/rendezvous.ts`.
fn close_session_line(session: SessionKey, code: u32, reason: &str) -> String {
    format!(
        "CLOSE_SESSION session={} code={code:#x} reason={reason:?}",
        session.0
    )
}

/// `interop/disconnect.ts`, `interop/quiet-peer.ts`.
fn session_closed_line(session: SessionKey) -> String {
    format!("SESSION_CLOSED session={}", session.0)
}

/// `interop/disconnect.ts`.
fn drain_ended_line(session: SessionKey, after_ms: u128, by: &str) -> String {
    format!(
        "DRAIN_ENDED session={} after_ms={after_ms} by={by}",
        session.0
    )
}

fn uni_envelope<C: Connection>(session: SessionKey, event: UniEvent) -> Envelope<C> {
    Envelope::Core(match event {
        UniEvent::Control(message) => Event::ControlMessage { session, message },
        UniEvent::SubgroupOpened { stream, header } => Event::DataStreamOpened {
            session,
            stream: StreamKey(stream),
            header,
        },
        UniEvent::Object { stream, object, .. } => Event::Object {
            session,
            stream: StreamKey(stream),
            object,
        },
        UniEvent::ObjectBegun {
            stream, object_id, ..
        } => Event::ObjectBegun {
            session,
            stream: StreamKey(stream),
            object_id,
        },
        UniEvent::TrackMalformed { stream, .. } => Event::TrackMalformed {
            session,
            stream: StreamKey(stream),
        },
        UniEvent::SubgroupClosed { stream, end, .. } => Event::DataStreamClosed {
            session,
            stream: StreamKey(stream),
            end,
        },
        UniEvent::FetchOpened { stream, header } => Event::FetchStreamOpened {
            session,
            stream: StreamKey(stream),
            request_id: header.request_id,
        },
        UniEvent::Failed(error) => Event::SessionFailed { session, error },
        UniEvent::ReaderPanicked => return reader_panicked(session),
    })
}

fn request_envelope<C: Connection>(
    session: SessionKey,
    event: PeerRequestStreamEvent<C::Send>,
    log: Option<&Log>,
) -> Envelope<C> {
    match event {
        PeerRequestStreamEvent::Opened {
            stream,
            send,
            first,
        } => Envelope::RequestOpened {
            session,
            stream: StreamKey(stream),
            send,
            first,
        },
        PeerRequestStreamEvent::Message { stream, message } => {
            Envelope::Core(Event::RequestMessage {
                session,
                stream: StreamKey(stream),
                message,
            })
        }
        PeerRequestStreamEvent::Closed { stream, how } => {
            // WHY a line for the reset as well as for STOP_SENDING: a peer
            // that cancels sends both (§3.3.2), and whichever the relay sees
            // first ends the request; the other is then not looked at.
            if let (PeerRequestClose::Reset(code), Some(log)) = (how, log) {
                log.print(reset_received_line(session, stream, code));
            }
            Envelope::Core(Event::RequestClosed {
                session,
                stream: StreamKey(stream),
                how: if how.is_fin() {
                    RequestEnd::Fin
                } else {
                    RequestEnd::Aborted
                },
            })
        }
        PeerRequestStreamEvent::Failed(error) => {
            Envelope::Core(Event::SessionFailed { session, error })
        }
        PeerRequestStreamEvent::ReaderPanicked => reader_panicked(session),
    }
}

/// `interop/no-answer.ts`.
fn reset_received_line(session: SessionKey, stream: u64, code: Option<u32>) -> String {
    format!(
        "RESET_RECEIVED session={} stream={stream} {}",
        session.0,
        stop_code(code)
    )
}

fn reader_panicked<C: Connection>(session: SessionKey) -> Envelope<C> {
    Envelope::Broken {
        session,
        code: TerminationCode::InternalError,
        reason: "a stream reader failed",
    }
}

/// `interop/no-answer.ts`.
fn connection_ended_line(session: SessionKey, end: &ConnectionEnd) -> String {
    format!("CONNECTION_ENDED session={} {end:?}", session.0)
}

/// Owns the readers of one session and reports the session closed.
///
/// WHY not at once when the connection goes: what had arrived before can
/// still be read (see the contract of `xmoq_moqt_io::RecvStream::read`), and
/// a publisher that sends its last Object and `PUBLISH_DONE` and then closes
/// is exactly that case. The readers put what they read in the same queue
/// this report goes to, so waiting for them keeps the order.
/// WHY a limit on the wait: a reader that is held back by a full queue must
/// not keep a dead session.
///
/// "Had arrived" means what the QUIC stack had received when the
/// WebTransport library read the publisher's `CLOSE_WEBTRANSPORT_SESSION`
/// capsule; stream data in a later packet is dropped below this code
/// (`web-transport-quinn` 0.13.2 `src/session.rs`, `quinn-proto` 0.11.19,
/// read on 2026-10-11). A publisher that needs its last Object to arrive
/// says `PUBLISH_DONE` and waits for the relay's FIN. Seen once in CI and in
/// 2 of 400 runs on a loaded machine
/// (`docs/experiments/2026-10-10-disconnects-through-own-relay.md`).
async fn watch_session<C: Connection>(
    session: SessionKey,
    connection: Arc<C>,
    mut readers: [IncomingStreams; 2],
    closed_read_timeout: Duration,
    log: Option<Log>,
    events: mpsc::Sender<Envelope<C>>,
) {
    let end = connection.closed().await;
    if let Some(log) = &log {
        log.print(connection_ended_line(session, &end));
    }
    let read = async {
        for reader in &mut readers {
            reader.done().await;
        }
    };
    let _ = tokio::time::timeout(closed_read_timeout, read).await;
    let _ = events
        .send(Envelope::Core(Event::SessionClosed { session }))
        .await;
}

/// How long after a `STOP_SENDING` on the relay's control stream the peer's
/// connection may end for the stop to count as part of its leaving.
/// Provisional.
const LEAVING_GRACE: Duration = Duration::from_millis(200);

/// Writes the relay's control stream of one session.
async fn write_control<C: Connection>(
    session: SessionKey,
    connection: Arc<C>,
    mut messages: mpsc::Receiver<ControlMessage>,
    trace: Trace,
    events: mpsc::Sender<Envelope<C>>,
) {
    // §3.3: the control stream stays open for the whole session, so it lives
    // as long as this task, which the session's handle ends.
    let Ok(mut control) = connection.open_uni().await else {
        return;
    };
    loop {
        let message = tokio::select! {
            message = messages.recv() => message,
            stopped = control.stopped() => {
                if let WriteError::Stopped(_) = stopped {
                    control_stopped(session, &*connection, &events).await;
                }
                return;
            }
        };
        let Some(message) = message else { return };
        match send_message(&mut control, &message, &trace, "control").await {
            Ok(()) => {}
            // WHY as the stop between writes: when the STOP_SENDING comes is
            // chance, and the peer's leaving and the code it is closed with
            // are not to depend on it.
            Err(xmoq_moqt_io::SendError::Write(WriteError::Stopped(_))) => {
                control_stopped(session, &*connection, &events).await;
                return;
            }
            // A lost connection is reported by the watcher of the session.
            Err(xmoq_moqt_io::SendError::Write(WriteError::Lost(_))) => return,
            Err(_) => {
                let broken = Envelope::Broken {
                    session,
                    code: TerminationCode::InternalError,
                    reason: "the control stream could not be written",
                };
                let _ = events.send(broken).await;
                return;
            }
        }
    }
}

/// The peer stopped the relay's control stream.
async fn control_stopped<C: Connection>(
    session: SessionKey,
    connection: &C,
    events: &mpsc::Sender<Envelope<C>>,
) {
    // WHY wait and see: an endpoint that goes away lets go of its streams,
    // which QUIC stacks turn into STOP_SENDING, and the close of its
    // connection can arrive a moment later. That is a peer leaving, and the
    // session then ends by itself, with what had arrived still read.
    let leaving = tokio::time::timeout(LEAVING_GRACE, connection.closed());
    if leaving.await.is_ok() {
        return;
    }
    // §3.3: closing a control stream is a PROTOCOL_VIOLATION, and a
    // STOP_SENDING closes this one.
    let broken = Envelope::Broken {
        session,
        code: TerminationCode::ProtocolViolation,
        reason: "the peer stopped the relay's control stream",
    };
    let _ = events.send(broken).await;
}

/// Writes the relay's half of one request stream, and watches for the peer
/// stopping it.
async fn write_request<C: Connection>(
    session: SessionKey,
    stream: StreamKey,
    mut send: C::Send,
    (mut commands, mut reset): (mpsc::Receiver<RequestCommand>, oneshot::Receiver<u32>),
    trace: Trace,
    log: Option<Log>,
    events: mpsc::Sender<Envelope<C>>,
) {
    let label = format!("bi#{}", stream.0);
    // The hub drops its sender with the writer's entry, without a reset.
    let mut may_be_reset = true;
    loop {
        let command = tokio::select! {
            // WHY biased, the reset first: the hub lets go of `commands` as
            // it sends the reset, and a `None` of `commands` that came first
            // would end the stream with a FIN, which the subscriber does not
            // read as the relay giving the request up.
            biased;
            code = &mut reset, if may_be_reset => {
                let Ok(code) = code else {
                    may_be_reset = false;
                    continue;
                };
                send.reset(code);
                return;
            }
            command = commands.recv() => command,
            stopped = send.stopped() => {
                if let WriteError::Stopped(code) = stopped {
                    if let Some(log) = &log {
                        log.print(stop_sending_received_line(session, stream, code));
                    }
                    // §3.3.2, §5.1: STOP_SENDING on a request stream cancels
                    // the request. The relay's half is reset in turn.
                    send.reset(stream_reset_code::CANCELLED);
                    let closed = Event::RequestClosed {
                        session,
                        stream,
                        how: RequestEnd::Aborted,
                    };
                    let _ = events.send(Envelope::Core(closed)).await;
                }
                return;
            }
        };
        match command {
            Some(RequestCommand::Send(message)) => {
                let of_a_namespace_subscription = matches!(
                    message,
                    ControlMessage::Namespace(_) | ControlMessage::NamespaceDone(_)
                );
                // An error means the peer stopped the stream or is gone, which
                // the other arm of the select reports. WHY the reset raced
                // with the write: a peer that gives no credit would hold it
                // back.
                let written = {
                    let write = send_message(&mut send, &message, &trace, &label);
                    tokio::pin!(write);
                    loop {
                        tokio::select! {
                            biased;
                            code = &mut reset, if may_be_reset => match code {
                                Ok(code) => break Err(code),
                                Err(_) => may_be_reset = false,
                            },
                            written = &mut write => break Ok(written),
                        }
                    }
                };
                let written = match written {
                    Ok(written) => written,
                    Err(code) => {
                        send.reset(code);
                        return;
                    }
                };
                // The relay sends the next ones of the SUBSCRIBE_NAMESPACE as
                // these are written.
                if of_a_namespace_subscription && written.is_ok() {
                    let event = Event::NamespaceWritten { session, stream };
                    let _ = events.send(Envelope::Core(event)).await;
                }
            }
            // `None`: the session is going.
            Some(RequestCommand::Finish) | None => {
                send.finish();
                return;
            }
        }
    }
}

/// Updates of a request the relay opened that came before the request was
/// written; they are written after it, in their order. The session lets no
/// more wait for their answer than `MAX_UNANSWERED_UPDATES`, which bounds
/// them.
struct Early {
    updates: VecDeque<ControlMessage>,
    /// Whether the hub still holds the sender of the updates.
    open: bool,
}

/// What became of opening a request stream and writing the request.
enum Opening<C: Connection> {
    Written(C::Send, C::Recv),
    /// The stream could not be opened, or the request not written.
    Unsent,
    /// The hub ended the request first: it was not sent, and the hub knows.
    Ended,
}

/// Opens the stream of a request the relay opens and writes the request.
/// Opening waits at the peer's stream limit, and writing while the peer
/// gives no flow control credit; an ending of the hub must get through, and
/// updates are kept for after the request. Before the request is written,
/// any ending is a cancel.
async fn open_and_send<C: Connection>(
    connection: &C,
    message: &ControlMessage,
    (updates, ending): (
        &mut mpsc::Receiver<ControlMessage>,
        &mut oneshot::Receiver<OwnRequestEnding>,
    ),
    early: &mut Early,
    (trace, label): (&Trace, &str),
) -> Opening<C> {
    let opened = {
        let open = connection.open_bi();
        tokio::pin!(open);
        loop {
            tokio::select! {
                biased;
                _ = &mut *ending => return Opening::Ended,
                update = updates.recv(), if early.open => match update {
                    Some(update) => early.updates.push_back(update),
                    None => early.open = false,
                },
                opened = &mut open => break opened,
            }
        }
    };
    let Ok((mut send, recv)) = opened else {
        return Opening::Unsent;
    };
    let written = {
        let write = send_message(&mut send, message, trace, label);
        tokio::pin!(write);
        loop {
            tokio::select! {
                biased;
                _ = &mut *ending => break None,
                update = updates.recv(), if early.open => match update {
                    Some(update) => early.updates.push_back(update),
                    None => early.open = false,
                },
                written = &mut write => break Some(written.is_ok()),
            }
        }
    };
    if written == Some(true) {
        return Opening::Written(send, recv);
    }
    send.reset(stream_reset_code::CANCELLED);
    recv.stop(stream_reset_code::CANCELLED);
    if written == Some(false) {
        Opening::Unsent
    } else {
        Opening::Ended
    }
}

/// Runs one request the relay opened: opens the stream, sends the request,
/// reports what the peer answers, and ends the stream the way the hub says.
async fn run_own_request<C: Connection>(
    session: SessionKey,
    request: OwnRequestKey,
    connection: Arc<C>,
    message: ControlMessage,
    (mut updates, mut ending): (
        mpsc::Receiver<ControlMessage>,
        oneshot::Receiver<OwnRequestEnding>,
    ),
    trace: Trace,
    events: mpsc::Sender<Envelope<C>>,
) {
    let label = format!("own#{}", request.0);
    let ended = |how| {
        Envelope::Core(Event::OwnRequestClosed {
            session,
            request,
            how,
        })
    };
    let mut early = Early {
        updates: VecDeque::new(),
        open: true,
    };
    let (mut send, recv) = match open_and_send(
        &*connection,
        &message,
        (&mut updates, &mut ending),
        &mut early,
        (&trace, &label),
    )
    .await
    {
        Opening::Written(send, recv) => (send, recv),
        Opening::Unsent => {
            let _ = events.send(ended(OwnRequestEnd::Unsent)).await;
            return;
        }
        Opening::Ended => return,
    };
    let mut reader = RequestReader::new(recv, &trace, &label);
    let (mut updates_written, ended_early) =
        match write_updates(&mut send, early.updates, (&trace, &label), &mut ending).await {
            UpdateWrite::Written => (true, None),
            UpdateWrite::Failed => (false, None),
            UpdateWrite::Ended(how) => (true, Some(how)),
        };
    let mut updates_open = early.open;
    // Each direction is reported once; after that only the other one, and the
    // hub's word, are waited for. What an ending means is the relay's to say.
    let (mut reading, mut watching) = (true, true);
    let how = loop {
        if let Some(how) = ended_early {
            break how;
        }
        if !updates_written {
            break None;
        }
        tokio::select! {
            biased;
            // An error: the session is going.
            how = &mut ending => break how.ok(),
            update = updates.recv(), if updates_open => match update {
                // WHY waited for here, and the peer's messages not read
                // meanwhile: a `REQUEST_UPDATE` is short, and a write in a
                // task of its own would let a `Finish` overtake it. The
                // ending is raced with it all the same, so that a `Cancel`
                // gets through a write the peer gives no credit for.
                Some(update) => {
                    match write_update(&mut send, &update, (&trace, &label), &mut ending).await {
                        UpdateWrite::Written => {}
                        UpdateWrite::Failed => updates_written = false,
                        UpdateWrite::Ended(how) => break how,
                    }
                }
                None => updates_open = false,
            },
            event = reader.next_event(), if reading => match event {
                Ok(RequestStreamEvent::Message(message)) => {
                    let event = Event::OwnRequestMessage { session, request, message };
                    if events.send(Envelope::Core(event)).await.is_err() {
                        break None;
                    }
                }
                Ok(RequestStreamEvent::Closed(how)) => {
                    reading = false;
                    let how = match how {
                        RequestClose::Fin => OwnRequestEnd::Fin,
                        RequestClose::Reset(_) | RequestClose::Stopped(_) => OwnRequestEnd::Reset,
                    };
                    let _ = events.send(ended(how)).await;
                }
                Err(RequestReadError::Peer(error)) => {
                    let failed = Event::SessionFailed { session, error };
                    let _ = events.send(Envelope::Core(failed)).await;
                    break None;
                }
                // A lost connection is reported by the watcher of the session.
                Err(RequestReadError::Lost) => break None,
            },
            stopped = send.stopped(), if watching => {
                watching = false;
                if let WriteError::Stopped(_) = stopped {
                    let _ = events.send(ended(OwnRequestEnd::Stopped)).await;
                }
            }
        }
    };
    if matches!(how, Some(OwnRequestEnding::Finish)) {
        // The updates the hub sent before it finished go first. WHY not
        // raced with anything: nothing is to overtake a FIN, and a session
        // that goes closes the connection, which ends the write.
        while let Ok(update) = updates.try_recv() {
            if send_message(&mut send, &update, &trace, &label)
                .await
                .is_err()
            {
                break;
            }
        }
    }
    end_own_request(send, reader, how);
}

/// What became of writing an update of a request the relay opened.
enum UpdateWrite {
    Written,
    /// The peer stopped the stream or is gone, which is reported apart.
    Failed,
    /// The hub ended the request first; `None`: the session is going.
    Ended(Option<OwnRequestEnding>),
}

/// Writes an update of a request the relay opened, unless the hub ends the
/// request first: a write the peer gives no credit for would hold a
/// `Cancel` back until the session ends.
async fn write_update<S: SendStream>(
    send: &mut S,
    update: &ControlMessage,
    (trace, label): (&Trace, &str),
    ending: &mut oneshot::Receiver<OwnRequestEnding>,
) -> UpdateWrite {
    let write = send_message(send, update, trace, label);
    tokio::pin!(write);
    tokio::select! {
        biased;
        how = &mut *ending => UpdateWrite::Ended(how.ok()),
        written = &mut write => match written {
            Ok(()) => UpdateWrite::Written,
            Err(_) => UpdateWrite::Failed,
        },
    }
}

/// [`write_update`] for each of `updates`, in their order, until one is
/// not written.
async fn write_updates<S: SendStream>(
    send: &mut S,
    updates: VecDeque<ControlMessage>,
    (trace, label): (&Trace, &str),
    ending: &mut oneshot::Receiver<OwnRequestEnding>,
) -> UpdateWrite {
    for update in updates {
        match write_update(send, &update, (trace, label), ending).await {
            UpdateWrite::Written => {}
            other => return other,
        }
    }
    UpdateWrite::Written
}

/// Ends the stream of a request the relay opened as the hub said; `None`:
/// the session is going, and the stream is not left half open.
fn end_own_request<S: SendStream, R>(
    send: S,
    reader: RequestReader<R>,
    how: Option<OwnRequestEnding>,
) where
    R: xmoq_moqt_io::RecvStream,
{
    let code = match how {
        Some(OwnRequestEnding::Finish) => {
            send.finish();
            reader.stop(stream_reset_code::CANCELLED);
            return;
        }
        Some(OwnRequestEnding::Cancel(code)) => code,
        None => stream_reset_code::CANCELLED,
    };
    send.reset(code);
    reader.stop(code);
}

/// `interop/fetch-rust.ts`.
fn fetch_pipe_ended_line(session: SessionKey, stream: StreamKey, how: FetchPipeEnd) -> String {
    format!(
        "FETCH_PIPE_ENDED session={} stream={} {}",
        session.0,
        stream.0,
        pipe_end(how)
    )
}

/// How the passing on of a fetch stream ended, for the log. The three waits
/// of a relayed FETCH show apart: the answer that did not come is a
/// `REQUEST_ERROR` with `TIMEOUT` and no pipe, a publisher that sent nothing
/// is `upstream_stalled`, a subscriber that took nothing `downstream_stalled`.
fn pipe_end(how: FetchPipeEnd) -> String {
    match how {
        FetchPipeEnd::Finished => "finished".to_string(),
        FetchPipeEnd::UpstreamAborted(code) => format!("upstream_reset {}", stop_code(code)),
        FetchPipeEnd::UpstreamLost => "upstream_lost".to_string(),
        FetchPipeEnd::UpstreamStalled => "upstream_stalled".to_string(),
        FetchPipeEnd::DownstreamStopped(code) => {
            format!("downstream_stopped {}", stop_code(code))
        }
        FetchPipeEnd::DownstreamStalled => "downstream_stalled".to_string(),
        FetchPipeEnd::DownstreamLost => "downstream_lost".to_string(),
    }
}

/// Bytes of a fetch stream written to the subscriber at a time. A subscriber
/// that does not take this much within its wait takes nothing, as far as the
/// relay goes.
const PIPE_PIECE: usize = 1024;

/// How long the two ends of a fetch stream that is passed on may keep the
/// relay waiting.
#[derive(Clone, Copy)]
struct PipeWaits {
    /// For bytes of the publisher, while the subscriber can take more.
    upstream: Duration,
    /// For the subscriber to take what was read.
    downstream: Duration,
}

/// One fetch stream of a publisher on its way to a subscriber.
///
/// WHY a task that reads and writes in turn, where a subgroup stream goes
/// through the relay's queues: a FETCH asks for what exists already, so the
/// publisher can send as fast as its link allows and the subscriber sets the
/// pace. A queue would be full at once, and a subscriber that is merely
/// slower than the publisher would lose its FETCH. Read only when the last
/// read is written, the publisher is held back by flow control instead.
/// What the relay holds of the stream then is the read in hand here and the
/// five the reader of the publisher's stream holds (`FetchBody`), 16 KiB
/// each: 96 KiB, next to the windows of the two connections.
///
/// The waits start anew with every read and every write, and a FETCH has no
/// time limit as a whole. How slowly a subscriber may read without being
/// cut is its transport's to say as well: the relay can write only as the
/// subscriber's side gives it room. On a real connection a slow subscriber
/// is cut much sooner than on an in-memory one: one that read 16 KiB every 2
/// seconds was cut as taking nothing, where one that read the same every
/// half second was not (`docs/experiments/2026-10-11-rust-client-fetch.md`).
/// Where the line lies was not measured.
struct FetchPipe<C: Connection> {
    connection: Arc<C>,
    header: FetchHeader,
    body: FetchBody,
    waits: PipeWaits,
    /// Where the stream to the subscriber stands among the streams to its
    /// session (`Action::PipeFetch`), and where the relay moves it to later.
    reordered: watch::Receiver<i32>,
    trace: Trace,
    label: String,
}

impl<C: Connection> FetchPipe<C> {
    /// Passes the stream on to its end. `None` when it was cut short from
    /// outside; then the one who cut it knows.
    async fn run(mut self, mut aborted: oneshot::Receiver<u32>) -> Option<FetchPipeEnd> {
        // An error means the hub dropped its end: the subscriber's session
        // is gone.
        let cut = |code: Result<u32, oneshot::error::RecvError>| {
            code.unwrap_or(stream_reset_code::SESSION_CLOSED)
        };
        let opened = tokio::select! {
            biased;
            _ = &mut aborted => return None,
            opened = tokio::time::timeout(
                self.waits.downstream,
                // Given up for a subscriber that gives the relay no stream:
                // the code its request stream is reset with for that.
                open_fetch_stream_in_order(
                    &*self.connection,
                    (self.header, *self.reordered.borrow_and_update()),
                    &self.trace,
                    &self.label,
                    stream_reset_code::TOO_FAR_BEHIND,
                ),
            ) => opened,
        };
        let mut send = match opened {
            Ok(Ok(send)) => send,
            Ok(Err(WriteError::Stopped(code))) => {
                return Some(FetchPipeEnd::DownstreamStopped(code));
            }
            Ok(Err(WriteError::Lost(_))) => return Some(FetchPipeEnd::DownstreamLost),
            // The subscriber gives the relay no stream to write on.
            Err(_) => return Some(FetchPipeEnd::DownstreamStalled),
        };
        loop {
            let chunk = tokio::select! {
                biased;
                code = &mut aborted => {
                    send.reset(cut(code));
                    return None;
                }
                // §5.2: the subscriber "MAY send STOP_SENDING for the data
                // stream" of a FETCH it cancels.
                stopped = send.stopped() => {
                    return Some(match stopped {
                        WriteError::Stopped(code) => {
                            // §5.2: a reset answers the STOP_SENDING of a
                            // subscriber that gives its FETCH up.
                            send.reset(stream_reset_code::CANCELLED);
                            FetchPipeEnd::DownstreamStopped(code)
                        }
                        WriteError::Lost(_) => FetchPipeEnd::DownstreamLost,
                    });
                }
                chunk = tokio::time::timeout(self.waits.upstream, self.body.next()) => chunk,
            };
            let bytes = match chunk {
                Ok(FetchChunk::Bytes(bytes)) => bytes,
                Ok(FetchChunk::End(StreamEnd::Fin)) => {
                    send.finish();
                    return Some(FetchPipeEnd::Finished);
                }
                Ok(FetchChunk::End(StreamEnd::Aborted(code))) => {
                    send.reset(xmoq_relay_core::fetch_reset_passed_on(code));
                    return Some(FetchPipeEnd::UpstreamAborted(code));
                }
                // The publisher is gone, or its readers are: what came next
                // will not be known (§3.3.3, UNKNOWN_OBJECT_STATUS).
                Ok(FetchChunk::End(StreamEnd::Lost) | FetchChunk::Stopped) => {
                    send.reset(stream_reset_code::UNKNOWN_OBJECT_STATUS);
                    return Some(FetchPipeEnd::UpstreamLost);
                }
                Err(_) => {
                    send.reset(stream_reset_code::UNKNOWN_OBJECT_STATUS);
                    return Some(FetchPipeEnd::UpstreamStalled);
                }
            };
            // WHY in pieces, each with the wait anew: the wait is for a
            // subscriber that takes nothing, and a write of a whole read
            // that does not end in time says only that it did not take all
            // of it. A subscriber on a slow link takes a piece now and then.
            for piece in bytes.chunks(PIPE_PIECE) {
                follow_send_order(&mut self.reordered, &mut send);
                let written = tokio::select! {
                    biased;
                    code = &mut aborted => {
                        send.reset(cut(code));
                        return None;
                    }
                    written = tokio::time::timeout(self.waits.downstream, send.write(piece)) => written,
                };
                match written {
                    Ok(Ok(())) => {}
                    Ok(Err(WriteError::Stopped(code))) => {
                        send.reset(stream_reset_code::CANCELLED);
                        return Some(FetchPipeEnd::DownstreamStopped(code));
                    }
                    Ok(Err(WriteError::Lost(_))) => return Some(FetchPipeEnd::DownstreamLost),
                    // WHY TOO_FAR_BEHIND, which §3.3.3 defines for a
                    // subscription: the draft has no code for a FETCH whose
                    // subscriber does not take it, and this one says what
                    // happened, that the receiver went over what the sender
                    // would hold for it.
                    Err(_) => {
                        send.reset(stream_reset_code::TOO_FAR_BEHIND);
                        return Some(FetchPipeEnd::DownstreamStalled);
                    }
                }
            }
        }
    }
}

/// The code of a `STOP_SENDING` or a reset for a log line. `none`: the
/// peer's code was outside the range WebTransport gives applications, or
/// there was none.
fn stop_code(code: Option<u32>) -> String {
    code.map_or_else(|| "code=none".to_string(), |code| format!("code={code:#x}"))
}

/// A peer stopped a request stream of its own. `interop/disconnect.ts` and
/// `interop/no-answer.ts` wait for this form, with `stream=`.
fn stop_sending_received_line(session: SessionKey, stream: StreamKey, code: Option<u32>) -> String {
    format!(
        "STOP_SENDING_RECEIVED session={} stream={} {}",
        session.0,
        stream.0,
        stop_code(code)
    )
}

/// [`HubConfig::log_arrivals`]: a subgroup stream or an Object of a peer the
/// hub takes, with the microseconds since 1970 then.
///
/// WHY a log of its own, and not [`HubConfig::log`]: there is a line for
/// every Object, which would crowd the other lines out of that log's queue.
/// The time is taken here, so a line that waits in the queue still says when
/// the hub took the Object; one that does not fit is dropped and counted
/// (`LOG_DROPPED`). WHY the Object's own fields only: the relay does not read
/// the payload's packaging (LOC); the publisher's line has the Timestamp.
fn log_arrival(arrivals: &Log, event: &Event) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_micros());
    match event {
        Event::DataStreamOpened {
            session,
            stream,
            header,
        } => arrivals.print(format!(
            "ARRIVED_STREAM session={} stream={} alias={} group={} subgroup={:?} at_us={now}",
            session.0, stream.0, header.track_alias, header.group_id, header.subgroup_id
        )),
        Event::Object {
            session,
            stream,
            object,
        } => arrivals.print(format!(
            "ARRIVED session={} stream={} object={} at_us={now}",
            session.0, stream.0, object.object_id
        )),
        _ => {}
    }
}

/// One line for the log about an action, for the actions that are rare enough
/// to log. An Object forwarded to a subscriber is not.
fn describe(action: &Action) -> Option<String> {
    Some(match action {
        // The interop run waits for it (`interop/rendezvous.ts`,
        // `interop/catalog.ts`): the one thing the relay does with a request
        // that none of its other lines shows.
        Action::SubscribeHeld { session, stream } => format!(
            "HOLD_REQUEST session={} stream={} SUBSCRIBE",
            session.0, stream.0
        ),
        Action::SendControl { session, message } => {
            format!("SEND session={} control {}", session.0, name(message))
        }
        Action::SendRequest {
            session,
            stream,
            message,
        } => {
            let kind = name(message);
            format!("SEND session={} stream={} {kind}", session.0, stream.0)
        }
        Action::FinishRequest { session, stream } => {
            format!("FINISH_REQUEST session={} stream={}", session.0, stream.0)
        }
        Action::ResetRequest {
            session,
            stream,
            code,
        } => format!(
            "RESET_REQUEST session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        // WHY the delivery timeout is not in the line: the interop runs wait
        // for these lines as they are.
        Action::OpenDataStream {
            session,
            out_stream,
            header,
            ..
        } => format!(
            "OPEN_DATA session={} out={} track_alias={} group={}",
            session.0, out_stream.0, header.track_alias, header.group_id
        ),
        Action::FinishDataStream { out_stream } => format!("FINISH_DATA out={}", out_stream.0),
        Action::ResetDataStream { out_stream, code } => {
            format!("RESET_DATA out={} code={code:#x}", out_stream.0)
        }
        Action::StopDataStream {
            session,
            stream,
            code,
        } => format!(
            "STOP_DATA session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        Action::StartDrainTimer {
            session,
            track_alias,
            ..
        } => format!(
            "DRAIN_TIMER session={} track_alias={track_alias}",
            session.0
        ),
        Action::StopRequest {
            session,
            stream,
            code,
        } => format!(
            "STOP_REQUEST session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        Action::OpenRequest { .. }
        | Action::SendOwnRequest { .. }
        | Action::FinishOwnRequest { .. }
        | Action::CancelOwnRequest { .. } => return describe_own_request(action),
        Action::PipeFetch { .. }
        | Action::AbortFetchPipe { .. }
        | Action::WriteFetchStream { .. }
        | Action::StartJoiningFetchTimer { .. }
        | Action::StartReceiptTimer { .. }
        | Action::NoteHeldBeyondCache { .. }
        | Action::SetFetchSendOrder { .. }
        | Action::AbortFetchWrite { .. } => return describe_fetch(action),
        // Logged where the session is closed, whoever asked for it.
        Action::CloseSession { .. }
        | Action::StartUpstreamTimer { .. }
        | Action::StartFetchStreamTimer { .. }
        | Action::StopUpstreamTimer { .. }
        | Action::StopDrainTimer { .. }
        | Action::StopEarlyTimer { .. }
        | Action::StopJoiningFetchTimer { .. }
        | Action::StopReceiptTimer { .. }
        | Action::SendObject { .. }
        | Action::SetDeliveryTimeout { .. }
        | Action::StartFetchEndTimer { .. }
        // Its SUBSCRIBE has a line where it was read (`HOLD_REQUEST`).
        | Action::StartRendezvousTimer { .. }
        | Action::StartAnnouncementTimer { .. }
        | Action::StartCheckTimer { .. }
        | Action::StartEarlyTimer { .. } => return None,
        Action::StartParentAnnouncementTimer { session, wait_ms } => format!(
            "ANNOUNCE_TO_PARENT_LATER session={} after_ms={wait_ms}",
            session.0
        ),
        Action::ConnectParent { uri, after_ms } => format!(
            "CONNECT_PARENT uri={} after_ms={after_ms}",
            printable(uri)
        ),
    })
}

/// The lines for the requests the relay opens.
fn describe_own_request(action: &Action) -> Option<String> {
    Some(match action {
        Action::OpenRequest {
            session,
            request,
            message,
        } => format!(
            "OPEN_REQUEST session={} own={} {}",
            session.0,
            request.0,
            name(message)
        ),
        Action::SendOwnRequest {
            session,
            request,
            message,
        } => format!(
            "SEND_OWN_REQUEST session={} own={} {}",
            session.0,
            request.0,
            name(message)
        ),
        Action::FinishOwnRequest { session, request } => {
            format!("FINISH_OWN_REQUEST session={} own={}", session.0, request.0)
        }
        Action::CancelOwnRequest {
            session,
            request,
            code,
        } => format!(
            "CANCEL_OWN_REQUEST session={} own={} code={code:#x}",
            session.0, request.0
        ),
        _ => return None,
    })
}

/// The log line of an action that is about a fetch stream.
fn describe_fetch(action: &Action) -> Option<String> {
    Some(match action {
        // The relay read a Joining FETCH before the SUBSCRIBE it names, and
        // holds it until that comes. The same word as for a SUBSCRIBE that
        // waits for a publisher (`Hub::feed`).
        Action::StartJoiningFetchTimer { session, stream } => format!(
            "HOLD_REQUEST session={} stream={} FETCH",
            session.0, stream.0
        ),
        // The relay holds a FETCH for an Object of its range that it is
        // receiving (`ReceiptWait` of `xmoq-relay-core`). WHY a word of its
        // own: the lines of `HOLD_REQUEST` are waited for by the interop
        // scenes of the Joining FETCH, and this one comes whenever a key
        // frame is on the way.
        Action::StartReceiptTimer {
            session,
            stream,
            wait_ms,
        } => format!(
            "HOLD_FETCH_FOR_OBJECT session={} stream={} wait_ms={wait_ms}",
            session.0, stream.0
        ),
        Action::PipeFetch {
            from,
            from_stream,
            session,
            stream,
            request_id,
            ..
        } => format!(
            "PIPE_FETCH session={} stream={} request_id={request_id} from_session={} from_stream={}",
            session.0, stream.0, from.0, from_stream.0
        ),
        Action::AbortFetchPipe {
            session,
            stream,
            code,
        } => format!(
            "ABORT_FETCH_PIPE session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        Action::WriteFetchStream {
            session,
            request_id,
            pieces,
            ..
        } => format!(
            "WRITE_FETCH_STREAM session={} request_id={request_id} pieces={}",
            session.0,
            pieces.len()
        ),
        Action::AbortFetchWrite {
            session,
            stream,
            code,
        } => format!(
            "ABORT_FETCH_WRITE session={} stream={} code={code:#x}",
            session.0, stream.0
        ),
        Action::SetFetchSendOrder {
            session,
            stream,
            send_order,
        } => format!(
            "FETCH_SEND_ORDER session={} stream={} send_order={send_order}",
            session.0, stream.0
        ),
        Action::NoteHeldBeyondCache {
            session,
            stream,
            refused,
            relay_bytes,
            session_bytes,
        } => format!(
            "HELD_BEYOND_CACHE session={} stream={} {} relay_bytes={relay_bytes} session_bytes={session_bytes}",
            session.0,
            stream.0,
            if *refused { "refused" } else { "stopped" }
        ),
        _ => return None,
    })
}

fn name(message: &ControlMessage) -> String {
    match message {
        ControlMessage::Setup(_) => "SETUP".to_string(),
        ControlMessage::RequestUpdate(_) => "REQUEST_UPDATE".to_string(),
        ControlMessage::Subscribe(_) => "SUBSCRIBE".to_string(),
        ControlMessage::TrackStatus(_) => "TRACK_STATUS".to_string(),
        ControlMessage::SubscribeOk(ok) => format!("SUBSCRIBE_OK track_alias={}", ok.track_alias),
        ControlMessage::Publish(_) => "PUBLISH".to_string(),
        ControlMessage::PublishNamespace(_) => "PUBLISH_NAMESPACE".to_string(),
        ControlMessage::RequestOk(_) => "REQUEST_OK".to_string(),
        ControlMessage::RequestError(error) => {
            format!("REQUEST_ERROR error_code={:#x}", error.error_code)
        }
        ControlMessage::PublishDone(done) => format!(
            "PUBLISH_DONE status_code={:#x} stream_count={}",
            done.status_code, done.stream_count
        ),
        ControlMessage::Fetch(_) => "FETCH".to_string(),
        ControlMessage::FetchOk(_) => "FETCH_OK".to_string(),
        ControlMessage::GoAway(go_away) => format!("GOAWAY timeout_ms={}", go_away.timeout_ms),
        ControlMessage::SubscribeNamespace(_) => "SUBSCRIBE_NAMESPACE".to_string(),
        ControlMessage::Namespace(suffix) => {
            format!("NAMESPACE suffix={}", printable_namespace(suffix))
        }
        ControlMessage::NamespaceDone(suffix) => {
            format!("NAMESPACE_DONE suffix={}", printable_namespace(suffix))
        }
        ControlMessage::UnsupportedRequest { message_type, .. } => {
            format!("unsupported request {message_type:#x}")
        }
    }
}

/// How many timers the session of `event` may have running for it to be
/// started, where it may have `max_timers_per_session` (`HubConfig`). A timer
/// that is not started is reported as run out at once (`Hub::set_timer`).
fn timer_limit(event: &Event, max_timers_per_session: usize) -> usize {
    // WHY the timer that gives a session an announcement back is
    // started whatever the session has running: one that is not started
    // counts as run out at once, which is right for a wait (what it
    // guarded is let go of) and wrong for this one, whose running out
    // gives the session something. A session with as many timers as it
    // may would have every announcement back at once, and the limit on
    // announcements would not hold for it. There is one such timer for
    // a session at most.
    // The same for the one after which an announcement to the parent is
    // sent again: one that ran out at once would send it again at once.
    match event {
        Event::AnnouncementTimeout { .. } | Event::ParentAnnouncementTimeout { .. } => usize::MAX,
        _ => max_timers_per_session,
    }
}

#[cfg(test)]
mod line_tests {
    use xmoq_moqt::data::{SubgroupHeader, SubgroupId};
    use xmoq_moqt::fetch::RangeEnd;
    use xmoq_moqt::message::{
        Fetch, FetchOk, FetchRange, PublishDone, ReasonPhrase, RequestError, RequestOk,
        RequestUpdate, SubscribeOk,
    };
    use xmoq_moqt::name::Location;

    use super::*;

    #[test]
    fn what_a_peer_named_stays_on_its_line_and_in_its_value() {
        let namespace = |fields: &[&[u8]]| {
            TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
        };
        let forged: &[&[u8]] = &[b"xmoq", b"a\nCLOSE_SESSION session=0 code=0x3"];
        let cases = [
            (
                Action::SendControl {
                    session: SessionKey(1),
                    message: ControlMessage::Namespace(namespace(forged)),
                },
                "SEND session=1 control NAMESPACE suffix=xmoq/a\\x0aCLOSE_SESSION\\x20session=0\\x20code=0x3",
            ),
            (
                Action::SendControl {
                    session: SessionKey(1),
                    message: ControlMessage::NamespaceDone(namespace(&[b"a/b"])),
                },
                "SEND session=1 control NAMESPACE_DONE suffix=a\\x2fb",
            ),
            (
                Action::ConnectParent {
                    uri: b"https://b.example\nSESSION_OPENED".to_vec(),
                    after_ms: 0,
                },
                "CONNECT_PARENT uri=https://b.example\\x0aSESSION_OPENED after_ms=0",
            ),
        ];
        for (action, line) in cases {
            assert_eq!(describe(&action).as_deref(), Some(line));
        }
    }

    // The interop scenes wait for these lines by their form, which a test of
    // the hub does not show (`interop/disconnect.ts`, at its top). Each row
    // names the scripts that wait for its kind.

    #[test]
    #[expect(clippy::too_many_lines, reason = "a table of 24 rows and one loop")]
    fn the_lines_of_actions_keep_the_form_the_interop_scenes_wait_for() {
        let s = SessionKey;
        let st = StreamKey;
        let fetch = ControlMessage::Fetch(Fetch {
            request_id: 2,
            range: FetchRange::RelativeJoining {
                joining_request_id: 0,
                joining_start: 0,
            },
            parameters: Vec::new(),
        });
        let update = ControlMessage::RequestUpdate(RequestUpdate {
            request_id: 4,
            parameters: Vec::new(),
        });
        let answer = |message| Action::SendRequest {
            session: s(1),
            stream: st(5),
            message,
        };
        let cases = [
            // catalog.ts, fetch-rust.ts, rendezvous.ts.
            (
                Action::SubscribeHeld {
                    session: s(1),
                    stream: st(5),
                },
                "HOLD_REQUEST session=1 stream=5 SUBSCRIBE",
            ),
            (
                Action::StartJoiningFetchTimer {
                    session: s(1),
                    stream: st(5),
                },
                "HOLD_REQUEST session=1 stream=5 FETCH",
            ),
            // half-object.ts.
            (
                Action::StartReceiptTimer {
                    session: s(1),
                    stream: st(5),
                    wait_ms: 300,
                },
                "HOLD_FETCH_FOR_OBJECT session=1 stream=5 wait_ms=300",
            ),
            // fetch-rust.ts, fetch.ts, half-object.ts.
            (
                Action::WriteFetchStream {
                    session: s(1),
                    stream: st(5),
                    request_id: 2,
                    pieces: vec![
                        FetchPiece::EndOfRange(
                            RangeEnd::Unknown,
                            Location {
                                group: 3,
                                object: 1,
                            },
                        ),
                        FetchPiece::EndOfRange(
                            RangeEnd::Unknown,
                            Location {
                                group: 4,
                                object: 0,
                            },
                        ),
                    ],
                    wait_ms: 1000,
                    send_order: 0,
                },
                "WRITE_FETCH_STREAM session=1 request_id=2 pieces=2",
            ),
            // fetch-rust.ts, fetch.ts.
            (
                Action::AbortFetchWrite {
                    session: s(1),
                    stream: st(5),
                    code: 0x1,
                },
                "ABORT_FETCH_WRITE session=1 stream=5 code=0x1",
            ),
            // quiet-peer.ts.
            (
                Action::NoteHeldBeyondCache {
                    session: s(1),
                    stream: st(5),
                    refused: false,
                    relay_bytes: 2048,
                    session_bytes: 1024,
                },
                "HELD_BEYOND_CACHE session=1 stream=5 stopped relay_bytes=2048 session_bytes=1024",
            ),
            (
                Action::NoteHeldBeyondCache {
                    session: s(1),
                    stream: st(5),
                    refused: true,
                    relay_bytes: 2048,
                    session_bytes: 1024,
                },
                "HELD_BEYOND_CACHE session=1 stream=5 refused relay_bytes=2048 session_bytes=1024",
            ),
            // auth-edge.ts, disconnect.ts, new-group.ts.
            (
                Action::OpenDataStream {
                    session: s(1),
                    out_stream: OutStreamKey(7),
                    header: SubgroupHeader {
                        track_alias: 9,
                        group_id: 3,
                        subgroup_id: SubgroupId::Zero,
                        publisher_priority: None,
                        properties: false,
                        end_of_group: false,
                        first_object: true,
                    },
                    subgroup_delivery_timeout: Some(500),
                    send_order: 0,
                },
                "OPEN_DATA session=1 out=7 track_alias=9 group=3",
            ),
            // disconnect.ts.
            (
                Action::FinishDataStream {
                    out_stream: OutStreamKey(7),
                },
                "FINISH_DATA out=7",
            ),
            // disconnect.ts.
            (
                Action::ResetDataStream {
                    out_stream: OutStreamKey(7),
                    code: 0x2,
                },
                "RESET_DATA out=7 code=0x2",
            ),
            // disconnect.ts.
            (
                Action::StopDataStream {
                    session: s(1),
                    stream: st(5),
                    code: 0x0,
                },
                "STOP_DATA session=1 stream=5 code=0x0",
            ),
            // disconnect.ts, rendezvous.ts.
            (
                Action::StartDrainTimer {
                    session: s(1),
                    track_alias: 9,
                    wait: 4,
                },
                "DRAIN_TIMER session=1 track_alias=9",
            ),
            // fetch-rust.ts.
            (
                Action::FinishRequest {
                    session: s(1),
                    stream: st(5),
                },
                "FINISH_REQUEST session=1 stream=5",
            ),
            // disconnect.ts.
            (
                Action::ResetRequest {
                    session: s(1),
                    stream: st(5),
                    code: 0x1,
                },
                "RESET_REQUEST session=1 stream=5 code=0x1",
            ),
            // chat-edge.ts, disconnect.ts, fetch-rust.ts, fetch.ts,
            // half-object.ts, no-answer.ts, rendezvous.ts.
            (
                Action::OpenRequest {
                    session: s(0),
                    request: OwnRequestKey(3),
                    message: fetch,
                },
                "OPEN_REQUEST session=0 own=3 FETCH",
            ),
            // video.ts.
            (
                Action::SendOwnRequest {
                    session: s(0),
                    request: OwnRequestKey(3),
                    message: update.clone(),
                },
                "SEND_OWN_REQUEST session=0 own=3 REQUEST_UPDATE",
            ),
            // chat-edge.ts, fetch.ts, no-answer.ts.
            (
                Action::CancelOwnRequest {
                    session: s(0),
                    request: OwnRequestKey(3),
                    code: 0x1,
                },
                "CANCEL_OWN_REQUEST session=0 own=3 code=0x1",
            ),
            // chat-edge.ts.
            (
                Action::StartParentAnnouncementTimer {
                    session: s(0),
                    wait_ms: 1500,
                },
                "ANNOUNCE_TO_PARENT_LATER session=0 after_ms=1500",
            ),
            // The control messages, by `name`: auth-edge.ts, chat-edge.ts,
            // disconnect.ts, fetch-rust.ts, half-object.ts, new-group.ts,
            // no-answer.ts, quiet-peer.ts, rendezvous.ts.
            (
                answer(ControlMessage::SubscribeOk(SubscribeOk {
                    track_alias: 9,
                    parameters: Vec::new(),
                    track_properties: Vec::new(),
                })),
                "SEND session=1 stream=5 SUBSCRIBE_OK track_alias=9",
            ),
            (
                answer(ControlMessage::RequestError(RequestError {
                    error_code: 0x10,
                    retry_interval: 0,
                    error_reason: ReasonPhrase::default(),
                    redirect: None,
                })),
                "SEND session=1 stream=5 REQUEST_ERROR error_code=0x10",
            ),
            (
                answer(ControlMessage::PublishDone(PublishDone {
                    status_code: 0x5,
                    stream_count: 2,
                    error_reason: ReasonPhrase::default(),
                })),
                "SEND session=1 stream=5 PUBLISH_DONE status_code=0x5 stream_count=2",
            ),
            (
                answer(ControlMessage::FetchOk(FetchOk {
                    end_of_track: false,
                    end_location: Location {
                        group: 4,
                        object: 0,
                    },
                    parameters: Vec::new(),
                    track_properties: Vec::new(),
                })),
                "SEND session=1 stream=5 FETCH_OK",
            ),
            (
                answer(ControlMessage::RequestOk(RequestOk::default())),
                "SEND session=1 stream=5 REQUEST_OK",
            ),
            (answer(update), "SEND session=1 stream=5 REQUEST_UPDATE"),
        ];
        for (action, line) in cases {
            assert_eq!(describe(&action).as_deref(), Some(line));
        }
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "a table of 28 rows and one loop")]
    fn the_lines_of_sessions_and_streams_keep_the_form_the_interop_scenes_wait_for() {
        let s = SessionKey;
        let st = StreamKey;
        let fetch_ok = ControlMessage::FetchOk(FetchOk {
            end_of_track: true,
            end_location: Location {
                group: 4,
                object: 0,
            },
            parameters: Vec::new(),
            track_properties: Vec::new(),
        });
        let going_away = |uri: &[u8]| GoingAway {
            new_session_uri: uri.to_vec(),
            timeout_ms: 10_000,
        };
        // (the line as the relay makes it; the line)
        let cases = [
            // harness.ts.
            (
                crate::listening_line("xmoq-relay/0.0.0", 4443),
                "LISTENING xmoq-relay/0.0.0 port=4443",
            ),
            // harness.ts.
            (log_dropped_line(12), "LOG_DROPPED lines=12"),
            // disconnect.ts, no-answer.ts, rendezvous.ts.
            (session_opened_line(s(3)), "SESSION_OPENED session=3"),
            // when-full.ts.
            (
                session_refused_line(s(3)),
                "SESSION_REFUSED session=3 too many sessions",
            ),
            // when-full.ts.
            (
                session_sent_away_line(s(3), "https://b.example:4443/?sent-away"),
                "SESSION_SENT_AWAY session=3 uri=https://b.example:4443/?sent-away",
            ),
            // auth-edge.ts, disconnect.ts, new-group.ts, no-answer.ts,
            // quiet-peer.ts, rendezvous.ts.
            (
                close_session_line(s(3), 0x11, "no SETUP in time"),
                "CLOSE_SESSION session=3 code=0x11 reason=\"no SETUP in time\"",
            ),
            // disconnect.ts, quiet-peer.ts.
            (session_closed_line(s(3)), "SESSION_CLOSED session=3"),
            // no-answer.ts.
            (
                connection_ended_line(
                    s(3),
                    &ConnectionEnd::ClosedByPeer {
                        code: 0,
                        reason: "bye".to_string(),
                    },
                ),
                "CONNECTION_ENDED session=3 ClosedByPeer { code: 0, reason: \"bye\" }",
            ),
            (
                connection_ended_line(s(3), &ConnectionEnd::ClosedLocally),
                "CONNECTION_ENDED session=3 ClosedLocally",
            ),
            (
                connection_ended_line(s(3), &ConnectionEnd::Lost("timed out".to_string())),
                "CONNECTION_ENDED session=3 Lost(\"timed out\")",
            ),
            // quiet-peer.ts, video.ts, watch-rust.ts.
            (
                going_away_line(2, &going_away(b"https://c.example:4443/")),
                "GOING_AWAY sessions=2 timeout_ms=10000 uri=https://c.example:4443/",
            ),
            (
                going_away_line(2, &going_away(b"")),
                "GOING_AWAY sessions=2 timeout_ms=10000 uri=",
            ),
            // watch-rust.ts.
            (stopping_line(2), "STOPPING sessions=2"),
            // disconnect.ts.
            (
                drain_ended_line(s(3), 250, "timeout"),
                "DRAIN_ENDED session=3 after_ms=250 by=timeout",
            ),
            (
                drain_ended_line(s(3), 40, "stop"),
                "DRAIN_ENDED session=3 after_ms=40 by=stop",
            ),
            // disconnect.ts, no-answer.ts.
            (
                stop_sending_received_line(s(3), st(4), Some(0x1)),
                "STOP_SENDING_RECEIVED session=3 stream=4 code=0x1",
            ),
            (
                stop_sending_received_line(s(3), st(4), None),
                "STOP_SENDING_RECEIVED session=3 stream=4 code=none",
            ),
            // no-answer.ts.
            (
                reset_received_line(s(3), 4, Some(0x1)),
                "RESET_RECEIVED session=3 stream=4 code=0x1",
            ),
            (
                reset_received_line(s(3), 4, None),
                "RESET_RECEIVED session=3 stream=4 code=none",
            ),
            // disconnect.ts, fetch.ts, rendezvous.ts.
            (
                recv_line(s(0), OwnRequestKey(3), &fetch_ok),
                "RECV session=0 own=3 FETCH_OK",
            ),
            // fetch-rust.ts.
            (
                fetch_pipe_ended_line(s(1), st(5), FetchPipeEnd::Finished),
                "FETCH_PIPE_ENDED session=1 stream=5 finished",
            ),
            (
                fetch_pipe_ended_line(s(1), st(5), FetchPipeEnd::UpstreamAborted(Some(0x6))),
                "FETCH_PIPE_ENDED session=1 stream=5 upstream_reset code=0x6",
            ),
            (
                fetch_pipe_ended_line(s(1), st(5), FetchPipeEnd::UpstreamLost),
                "FETCH_PIPE_ENDED session=1 stream=5 upstream_lost",
            ),
            (
                fetch_pipe_ended_line(s(1), st(5), FetchPipeEnd::UpstreamStalled),
                "FETCH_PIPE_ENDED session=1 stream=5 upstream_stalled",
            ),
            (
                fetch_pipe_ended_line(s(1), st(5), FetchPipeEnd::DownstreamStopped(None)),
                "FETCH_PIPE_ENDED session=1 stream=5 downstream_stopped code=none",
            ),
            (
                fetch_pipe_ended_line(s(1), st(5), FetchPipeEnd::DownstreamStalled),
                "FETCH_PIPE_ENDED session=1 stream=5 downstream_stalled",
            ),
            (
                fetch_pipe_ended_line(s(1), st(5), FetchPipeEnd::DownstreamLost),
                "FETCH_PIPE_ENDED session=1 stream=5 downstream_lost",
            ),
        ];
        for (made, line) in cases {
            assert_eq!(made, line);
        }
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    fn timeout(session: u64, stream: u64) -> Event {
        Event::RequestTimeout {
            session: SessionKey(session),
            stream: StreamKey(stream),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_timer_for_a_peer_to_end_a_request_stream_goes_when_it_did() {
        let mut timers = Timers::default();
        let wait = Duration::from_secs(2);
        for stream in 0..3 {
            assert!(
                timers
                    .start(SessionKey(1), 3, wait, timeout(1, stream))
                    .is_none()
            );
        }
        // The peer ended the stream 1: its timer is gone, and makes room.
        timers.stop_request_close(SessionKey(1), StreamKey(1));
        assert_eq!(timers.len(), 2);
        assert!(
            timers
                .start(SessionKey(1), 3, wait, timeout(1, 3))
                .is_none()
        );
        // A stream that has none, and one of another session with that
        // number: nothing.
        timers.stop_request_close(SessionKey(1), StreamKey(1));
        timers.stop_request_close(SessionKey(2), StreamKey(0));
        assert_eq!(timers.len(), 3);
        tokio::time::sleep(wait).await;
        assert_eq!(
            timers.take_due(),
            [timeout(1, 0), timeout(1, 2), timeout(1, 3)]
        );
        // One that fired is not counted down a second time, and is not
        // remembered any longer.
        assert_eq!(timers.remembered(), 0);
        timers.stop_request_close(SessionKey(1), StreamKey(0));
        assert!(
            timers
                .start(SessionKey(1), 1, wait, timeout(1, 4))
                .is_none()
        );
        // Nor is one of a session that is gone.
        assert_eq!(timers.remembered(), 1);
        timers.forget_session(SessionKey(1));
        assert_eq!((timers.len(), timers.remembered()), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_for_a_subscriber_to_end_a_served_fetch_is_dropped_when_it_does() {
        // The expectations below are spelled out for a limit of 1024.
        let served = |stream| Event::FetchEndTimeout {
            session: SessionKey(1),
            stream: StreamKey(stream),
        };
        let mut timers = Timers::default();
        let wait = Duration::from_secs(3);
        // A subscriber that ends its direction of every FETCH it was
        // served holds no timer, however many it fetches in the 3 seconds.
        for stream in 0..2048 {
            assert!(
                timers
                    .start(SessionKey(1), 1024, wait, served(stream))
                    .is_none(),
                "{stream}"
            );
            timers.stop_request_close(SessionKey(1), StreamKey(stream));
        }
        assert_eq!((timers.len(), timers.remembered()), (0, 0));
        // One that does not is found when the time is over, and the timer
        // that follows for the same stream is one of its own.
        assert!(timers.start(SessionKey(1), 1024, wait, served(7)).is_none());
        tokio::time::sleep(wait).await;
        assert_eq!(timers.take_due(), [served(7)]);
        assert_eq!((timers.len(), timers.remembered()), (0, 0));
        assert!(
            timers
                .start(SessionKey(1), 1024, wait, timeout(1, 7))
                .is_none()
        );
        timers.stop_request_close(SessionKey(1), StreamKey(7));
        assert_eq!((timers.len(), timers.remembered()), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn the_waits_of_an_early_stream_and_of_a_held_joining_fetch_are_dropped_when_settled() {
        // The expectations below are spelled out for a limit of 1024.
        let early = |stream| Event::EarlyStreamTimeout {
            session: SessionKey(1),
            stream: StreamKey(stream),
        };
        let joining = |stream| Event::JoiningFetchTimeout {
            session: SessionKey(1),
            stream: StreamKey(stream),
        };
        let mut timers = Timers::default();
        let wait = Duration::from_secs(2);
        // A publisher whose early streams are all settled, and a subscriber
        // whose Joining FETCH requests all see their SUBSCRIBE come, hold no
        // timer, however many come in the 2 seconds.
        for stream in 0..2048 {
            assert!(
                timers
                    .start(SessionKey(1), 1024, wait, early(stream))
                    .is_none(),
                "{stream}"
            );
            timers.stop_early(SessionKey(1), StreamKey(stream));
            assert!(
                timers
                    .start(SessionKey(1), 1024, wait, joining(stream))
                    .is_none(),
                "{stream}"
            );
            timers.stop_joining(SessionKey(1), StreamKey(stream));
        }
        assert_eq!(
            (timers.len(), timers.early.len(), timers.joining.len()),
            (0, 0, 0)
        );
        // One that is not settled runs out, and is not remembered after.
        assert!(timers.start(SessionKey(1), 1024, wait, early(7)).is_none());
        assert!(
            timers
                .start(SessionKey(1), 1024, wait, joining(8))
                .is_none()
        );
        tokio::time::sleep(wait).await;
        assert_eq!(timers.take_due(), [early(7), joining(8)]);
        assert_eq!(
            (timers.len(), timers.early.len(), timers.joining.len()),
            (0, 0, 0)
        );
        // A stop after it ran out takes nothing from another timer.
        assert!(timers.start(SessionKey(1), 1, wait, early(9)).is_none());
        timers.stop_early(SessionKey(1), StreamKey(7));
        timers.stop_joining(SessionKey(1), StreamKey(8));
        assert_eq!(timers.len(), 1);
        assert!(timers.start(SessionKey(1), 1, wait, early(10)).is_some());
    }

    #[test]
    fn every_timer_counts_against_the_limit_but_the_two_that_give_something_back() {
        let session = SessionKey(1);
        // (the event of the timer; the limit for a session that may have 8)
        let cases = [
            // A wait for the last streams after PUBLISH_DONE: it ends at once
            // where it is not started.
            (
                Event::DrainTimeout {
                    session,
                    track_alias: 2,
                    wait: 0,
                },
                8,
            ),
            // An early stream: it is dropped at once.
            (
                Event::EarlyStreamTimeout {
                    session,
                    stream: StreamKey(3),
                },
                8,
            ),
            // A request the relay sent: it counts as unanswered at once.
            (
                Event::UpstreamTimeout {
                    session,
                    request: OwnRequestKey(0),
                },
                8,
            ),
            (timeout(1, 4), 8),
            (Event::AnnouncementTimeout { session }, usize::MAX),
            (Event::ParentAnnouncementTimeout { session }, usize::MAX),
        ];
        for (event, limit) in cases {
            assert_eq!(timer_limit(&event, 8), limit, "{event:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_gets_only_so_many_timers() {
        // The expectations below are spelled out for a limit of 1024.
        let mut timers = Timers::default();
        let wait = Duration::from_secs(2);
        for stream in 0..1024 {
            assert!(
                timers
                    .start(SessionKey(1), 1024, wait, timeout(1, stream))
                    .is_none()
            );
        }
        assert_eq!(
            timers.start(SessionKey(1), 1024, wait, timeout(1, 1024)),
            Some(timeout(1, 1024))
        );
        // Another session is not held back by this one.
        assert!(
            timers
                .start(SessionKey(2), 1024, wait, timeout(2, 0))
                .is_none()
        );
        assert_eq!(timers.len(), 1025);

        // A timer that fired makes room for a new one.
        tokio::time::sleep(wait).await;
        timers.due().await;
        assert_eq!(timers.take_due().len(), 1025);
        assert_eq!(timers.len(), 0);
        assert!(
            timers
                .start(SessionKey(1), 1024, wait, timeout(1, 1025))
                .is_none()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn timers_fire_in_the_order_they_are_due_and_go_with_their_session() {
        let mut timers = Timers::default();
        assert!(
            timers
                .start(SessionKey(1), 8, Duration::from_secs(3), timeout(1, 0))
                .is_none()
        );
        assert!(
            timers
                .start(SessionKey(2), 8, Duration::from_secs(1), timeout(2, 0))
                .is_none()
        );
        assert!(
            timers
                .start(SessionKey(1), 8, Duration::from_secs(2), timeout(1, 1))
                .is_none()
        );

        // Nothing is due yet.
        assert_eq!(timers.take_due(), []);
        timers.due().await;
        assert_eq!(timers.take_due(), [timeout(2, 0)]);

        timers.forget_session(SessionKey(1));
        assert_eq!(timers.len(), 0);
        // With nothing running, there is nothing to wait for.
        let waited = tokio::time::timeout(Duration::from_secs(60), timers.due()).await;
        assert!(waited.is_err());
        // The session starts from nothing again.
        assert!(
            timers
                .start(SessionKey(1), 8, Duration::from_secs(1), timeout(1, 2))
                .is_none()
        );
        assert_eq!(timers.len(), 1);
    }
}

#[cfg(test)]
mod fetch_write_tests {
    use std::sync::atomic::Ordering as Ordering_;
    use xmoq_moqt::fetch::{FetchEvent, FetchStreamDecoder, RangeEnd};
    use xmoq_moqt::name::Location;

    use xmoq_moqt_io::ReadError;
    use xmoq_moqt_io::memory::{MemoryConnection, pair, pair_with_window};

    use super::*;

    const IN_TIME: Duration = Duration::from_secs(5);
    /// Longer than any test but the one about it takes.
    const WHOLE: Duration = Duration::from_secs(3600);

    /// An End of Unknown Range indicator of 20 bytes: its Group and its
    /// Object ID take nine bytes each.
    fn indicator(n: u64) -> FetchPiece {
        let location = Location {
            group: u64::MAX - 1_000 + n,
            object: u64::MAX,
        };
        FetchPiece::EndOfRange(RangeEnd::Unknown, location)
    }

    /// The same as a fetch stream gives it.
    fn read(piece: FetchPiece) -> FetchEvent {
        match piece {
            FetchPiece::EndOfRange(kind, location) => FetchEvent::EndOfRange { kind, location },
            FetchPiece::Object(cached) => FetchEvent::Object(FetchObject {
                group_id: cached.group_id,
                subgroup_id: Some(cached.subgroup_id),
                object_id: cached.object.object_id,
                publisher_priority: cached.publisher_priority,
                properties: cached.object.properties.clone(),
                payload: cached.object.payload.clone(),
            }),
        }
    }

    /// A write of `pieces`, and the connection, which the hub holds for as
    /// long as the session lasts: it is closed when nobody holds it.
    fn write(
        connection: MemoryConnection,
        pieces: Vec<FetchPiece>,
        patience: Duration,
    ) -> (FetchWrite<MemoryConnection>, Arc<MemoryConnection>) {
        let connection = Arc::new(connection);
        let write = FetchWrite {
            connection: Arc::clone(&connection),
            header: FetchHeader { request_id: 4 },
            send_order: watch::channel(0).1,
            pieces,
            patience,
            whole: WHOLE,
            trace: Trace::default(),
            label: "test".to_string(),
        };
        (write, connection)
    }

    /// Reads a fetch stream to its end, waiting `pause` before every read:
    /// what it carried, and the error it ended with, `None` for a FIN.
    async fn read_all(
        mut stream: impl RecvStream,
        pause: Duration,
    ) -> (Vec<FetchEvent>, Option<ReadError>) {
        let mut decoder = FetchStreamDecoder::new(64 * 1024, GroupOrder::Ascending);
        let mut events = Vec::new();
        let mut buffer = [0u8; 64];
        loop {
            tokio::time::sleep(pause).await;
            match stream.read(&mut buffer).await {
                Ok(Some(read)) => {
                    decoder.push(&buffer[..read]).unwrap();
                    while let Some(event) = decoder.next_event().unwrap() {
                        events.push(event);
                    }
                }
                Ok(None) => return (events, None),
                Err(error) => return (events, Some(error)),
            }
        }
    }

    async fn in_time<T>(what: impl Future<Output = T>) -> T {
        tokio::time::timeout(IN_TIME, what).await.expect("in time")
    }

    #[tokio::test]
    async fn the_pieces_are_written_in_their_order_and_the_stream_is_finished() {
        let (relay, subscriber) = pair();
        let small = FetchPiece::EndOfRange(
            RangeEnd::Unknown,
            Location {
                group: 2,
                object: 7,
            },
        );
        let pieces = vec![small, indicator(0), indicator(1)];
        let (write, _session) = write(relay, pieces.clone(), IN_TIME);
        // Nobody can order it stopped.
        let writing = tokio::spawn(write.run(oneshot::channel().1));
        let stream = in_time(subscriber.accept_uni()).await.unwrap();
        let (events, end) = in_time(read_all(stream, Duration::ZERO)).await;
        let mut expected = vec![FetchEvent::Header(FetchHeader { request_id: 4 })];
        expected.extend(pieces.into_iter().map(read));
        assert_eq!((events, end), (expected, None));
        assert_eq!(in_time(writing).await.unwrap().given_up(), None);
    }

    #[tokio::test]
    async fn a_subscriber_that_takes_the_pieces_slowly_gets_all_of_them() {
        // The window lets 64 bytes wait; 50 indicators of 20 bytes.
        let (relay, subscriber) = pair_with_window(64);
        let pieces: Vec<FetchPiece> = (0..50).map(indicator).collect();
        // A second for each piece, and the subscriber takes 64 bytes every
        // 100 ms: no piece waits for a second, and all of them take more.
        let patience = Duration::from_secs(1);
        let started = Instant::now();
        let (write, _session) = write(relay, pieces.clone(), patience);
        let writing = tokio::spawn(write.run(oneshot::channel().1));
        let stream = in_time(subscriber.accept_uni()).await.unwrap();
        let (events, end) = in_time(read_all(stream, Duration::from_millis(100))).await;
        let mut expected = vec![FetchEvent::Header(FetchHeader { request_id: 4 })];
        expected.extend(pieces.into_iter().map(read));
        assert_eq!((events, end), (expected, None));
        assert_eq!(in_time(writing).await.unwrap().given_up(), None);
        assert!(started.elapsed() > patience, "{:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_time_for_the_whole_that_no_clock_holds_is_as_good_as_none() {
        let (relay, subscriber) = pair();
        let connection = Arc::new(relay);
        let write = FetchWrite {
            connection: Arc::clone(&connection),
            header: FetchHeader { request_id: 4 },
            send_order: watch::channel(0).1,
            pieces: vec![indicator(0)],
            patience: IN_TIME,
            whole: Duration::MAX,
            trace: Trace::default(),
            label: "test".to_string(),
        };
        let writing = tokio::spawn(write.run(oneshot::channel().1));
        let stream = in_time(subscriber.accept_uni()).await.unwrap();
        let (events, end) = in_time(read_all(stream, Duration::ZERO)).await;
        assert_eq!((events.len(), end), (2, None));
        assert_eq!(in_time(writing).await.unwrap().given_up(), None);
    }

    #[test]
    fn the_whole_of_a_fetch_stream_has_the_least_time_or_its_bytes_at_the_least_rate() {
        // 30 s at the least, and 500 kbit/s at the least.
        let config = HubConfig::default();
        // (bytes of payload, the time for the whole in ms)
        let cases = [
            (0, 30_000),
            // 320 kB take 5.1 s at that rate: the least time.
            (320_000, 30_000),
            // 1,875,000 bytes take 30 s exactly.
            (1_875_000, 30_000),
            (1_875_063, 30_001),
            // 8 MiB.
            (8_388_608, 134_217),
        ];
        for (bytes, whole_ms) in cases {
            assert_eq!(
                config.cached_fetch_whole(bytes),
                Duration::from_millis(whole_ms),
                "{bytes}"
            );
        }
        // A time nobody can wait for is no panic.
        let forever = HubConfig {
            cached_fetch_whole_ms: u64::MAX,
            ..HubConfig::default()
        };
        assert_eq!(
            forever.cached_fetch_whole(1),
            Duration::from_millis(u64::MAX)
        );
    }

    #[tokio::test]
    async fn a_stream_the_relay_writes_takes_the_send_order_it_is_moved_to() {
        // The window lets 64 bytes wait, so the write of 50 indicators of
        // 20 bytes waits for the subscriber while the order is changed.
        let (relay, subscriber) = pair_with_window(64);
        let connection = Arc::new(relay);
        let (moved, reordered) = watch::channel(-7);
        let write = FetchWrite {
            connection: Arc::clone(&connection),
            header: FetchHeader { request_id: 4 },
            send_order: reordered,
            pieces: (0..50).map(indicator).collect(),
            patience: IN_TIME,
            whole: WHOLE,
            trace: Trace::default(),
            label: "test".to_string(),
        };
        let writing = tokio::spawn(write.run(oneshot::channel().1));
        let mut stream = in_time(subscriber.accept_uni()).await.unwrap();
        // The order is set before the header is written.
        let mut buffer = [0u8; 64];
        assert!(in_time(stream.read(&mut buffer)).await.unwrap().is_some());
        assert_eq!(stream.send_order(), Some(-7));
        moved.send_replace(-897);
        while in_time(stream.read(&mut buffer)).await.unwrap().is_some() {}
        assert_eq!(stream.send_order(), Some(-897));
        assert_eq!(in_time(writing).await.unwrap().given_up(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscriber_that_takes_each_piece_in_time_and_the_whole_too_slowly_is_cut() {
        // The window lets 64 bytes wait; 50 indicators of 20 bytes. The
        // subscriber takes 64 bytes every 100 ms, so no piece waits for the
        // second it has, and the whole would take about 1.6 s.
        // (the time for the whole, in ms; whether the stream is finished)
        for (whole_ms, finished) in [(3_000, true), (1_000, false)] {
            let (relay, subscriber) = pair_with_window(64);
            let pieces: Vec<FetchPiece> = (0..50).map(indicator).collect();
            let connection = Arc::new(relay);
            let write = FetchWrite {
                connection: Arc::clone(&connection),
                header: FetchHeader { request_id: 4 },
                send_order: watch::channel(0).1,
                pieces,
                patience: Duration::from_secs(1),
                whole: Duration::from_millis(whole_ms),
                trace: Trace::default(),
                label: "test".to_string(),
            };
            let started = Instant::now();
            let writing = tokio::spawn(write.run(oneshot::channel().1));
            let stream = subscriber.accept_uni().await.unwrap();
            let (events, end) = read_all(stream, Duration::from_millis(100)).await;
            let written = writing.await.unwrap();
            if finished {
                assert_eq!((events.len(), end, written.given_up()), (51, None, None));
            } else {
                // TOO_FAR_BEHIND (0x5), a second after it began.
                assert_eq!(written.how, WriteEnd::TooSlow);
                assert_eq!(written.given_up(), Some(0x5));
                assert_eq!(end, Some(ReadError::Reset(Some(0x5))));
                assert!(events.len() < 51, "{}", events.len());
                assert!(started.elapsed() < Duration::from_millis(1_500));
            }
        }
    }

    #[tokio::test]
    async fn a_write_that_waits_goes_on_when_nobody_is_left_to_order_it_stopped() {
        // The window lets 8 bytes wait, and a piece is 20: the first write
        // waits with a part of the piece written. Nobody can order the
        // stream stopped, which the writer learns then or before; a write
        // begun anew would write that part twice. Which of the two the
        // writer sees first is drawn, so many times.
        for run in 0..40 {
            let (relay, subscriber) = pair_with_window(8);
            let pieces = vec![indicator(0), indicator(1)];
            let (write, _session) = write(relay, pieces.clone(), IN_TIME);
            let writing = tokio::spawn(write.run(oneshot::channel().1));
            let stream = in_time(subscriber.accept_uni()).await.unwrap();
            let (events, end) = in_time(read_all(stream, Duration::ZERO)).await;
            let mut expected = vec![FetchEvent::Header(FetchHeader { request_id: 4 })];
            expected.extend(pieces.into_iter().map(read));
            assert_eq!((events, end), (expected, None), "run {run}");
            assert_eq!(in_time(writing).await.unwrap().given_up(), None);
        }
    }

    /// Reads a stream to where it ends: `None` for a FIN.
    async fn end_of(mut stream: impl RecvStream) -> Option<ReadError> {
        let mut buffer = [0u8; 64];
        loop {
            match in_time(stream.read(&mut buffer)).await {
                Ok(Some(_)) => {}
                Ok(None) => return None,
                Err(error) => return Some(error),
            }
        }
    }

    #[tokio::test]
    async fn a_stream_that_is_not_written_to_its_end_is_reset_with_the_code_of_what_ended_it() {
        // TOO_FAR_BEHIND (0x5), MALFORMED_TRACK (0x12), CANCELLED (0x1).
        // (the order to stop that comes while the writer waits in a piece,
        //  `None` where the subscriber just takes nothing; the code the
        //  stream is given up with, which the subscriber reads as the reset)
        let cases = [(None, 0x5), (Some(0x12), 0x12), (Some(0x1), 0x1)];
        for (order, code) in cases {
            // More than the window of 64 bytes lets wait.
            let (relay, subscriber) = pair_with_window(64);
            let pieces = (0..50).map(indicator).collect();
            let patience = match order {
                None => Duration::from_millis(200),
                Some(_) => IN_TIME,
            };
            let (orders, ordered) = oneshot::channel();
            let (write, _session) = write(relay, pieces, patience);
            let writing = tokio::spawn(write.run(ordered));
            let stream = in_time(subscriber.accept_uni()).await.unwrap();
            if let Some(with) = order {
                // The writer waits in a piece by now or soon; the order is
                // taken wherever it waits.
                tokio::time::sleep(Duration::from_millis(50)).await;
                orders.send(with).unwrap();
            }
            assert_eq!(
                in_time(writing).await.unwrap().given_up(),
                Some(code),
                "{order:?}"
            );
            // What was written and not read is gone with the reset.
            assert_eq!(
                end_of(stream).await,
                Some(ReadError::Reset(Some(code))),
                "{order:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_order_to_stop_that_is_there_before_the_stream_is_opened_leaves_none() {
        // The order and the stream that can be opened at once are both
        // ready: which of two such things is taken is drawn, unless one is
        // looked at first. Many times.
        for run in 0..50 {
            let (relay, subscriber) = pair();
            let (write, _session) = write(relay, vec![indicator(0)], IN_TIME);
            let (orders, ordered) = oneshot::channel();
            // MALFORMED_TRACK (0x12), before the task has run once.
            orders.send(0x12).unwrap();
            assert_eq!(
                in_time(write.run(ordered)).await.given_up(),
                Some(0x12),
                "run {run}"
            );
            // No stream was opened: nothing is there to reset.
            let opened = tokio::time::timeout(Duration::from_millis(20), subscriber.accept_uni());
            assert!(opened.await.is_err(), "run {run}");
        }
    }

    #[tokio::test]
    async fn an_order_to_stop_that_comes_while_the_header_waits_resets_with_its_code() {
        // The window lets one byte wait, and the header is two: the stream
        // is open, and its header is not written.
        let (relay, subscriber) = pair_with_window(1);
        let (write, _session) = write(relay, vec![indicator(0)], IN_TIME);
        let (orders, ordered) = oneshot::channel();
        let writing = tokio::spawn(write.run(ordered));
        let stream = in_time(subscriber.accept_uni()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // MALFORMED_TRACK (0x12): the code on the wire, and the one that is
        // reported.
        orders.send(0x12).unwrap();
        assert_eq!(in_time(writing).await.unwrap().given_up(), Some(0x12));
        assert_eq!(end_of(stream).await, Some(ReadError::Reset(Some(0x12))));
    }

    #[tokio::test]
    async fn an_order_to_stop_that_comes_with_the_end_of_the_last_write_is_kept() {
        // The window lets 16 bytes wait: the header, 2 bytes, and 14 of the
        // 20 of the one piece. The write waits for the last 6.
        for run in 0..50 {
            let (relay, subscriber) = pair_with_window(16);
            let (write, _session) = write(relay, vec![indicator(0)], IN_TIME);
            let (orders, ordered) = oneshot::channel();
            let writing = tokio::spawn(write.run(ordered));
            let mut stream = in_time(subscriber.accept_uni()).await.unwrap();
            let mut buffer = [0u8; 16];
            let mut read = 0;
            while read < 16 {
                read += in_time(stream.read(&mut buffer[read..]))
                    .await
                    .unwrap()
                    .unwrap();
            }
            // Now the write can complete, and the order is there as well,
            // before the writer runs again: CANCELLED (0x1).
            orders.send(0x1).unwrap();
            assert_eq!(
                in_time(writing).await.unwrap().given_up(),
                Some(0x1),
                "run {run}"
            );
            assert_eq!(
                end_of(stream).await,
                Some(ReadError::Reset(Some(0x1))),
                "run {run}"
            );
        }
    }

    /// What sends an order to stop when a stream has taken its write
    /// number `after`, before the writer runs on.
    #[derive(Clone)]
    struct OrderAfter {
        after: usize,
        written: Arc<std::sync::atomic::AtomicUsize>,
        order: Arc<std::sync::Mutex<Option<oneshot::Sender<u32>>>>,
    }

    /// A sending half that orders its own stream stopped ([`OrderAfter`]).
    struct Ordering(<MemoryConnection as Connection>::Send, OrderAfter);

    impl SendStream for Ordering {
        async fn write_all(&mut self, bytes: &[u8]) -> Result<(), WriteError> {
            let written = self.0.write_all(bytes).await;
            let count = self.1.written.fetch_add(1, Ordering_::SeqCst) + 1;
            if count == self.1.after
                && let Some(order) = self.1.order.lock().unwrap().take()
            {
                // MALFORMED_TRACK (0x12).
                order.send(0x12).unwrap();
            }
            written
        }

        fn finish(self) {
            self.0.finish();
        }

        async fn finish_and_wait(self) -> Result<(), WriteError> {
            self.0.finish_and_wait().await
        }

        fn reset(self, code: u32) {
            self.0.reset(code);
        }

        async fn stopped(&mut self) -> WriteError {
            self.0.stopped().await
        }
    }

    /// A connection whose streams are [`Ordering`].
    struct Orderly(MemoryConnection, OrderAfter);

    impl Connection for Orderly {
        type Send = Ordering;
        type Recv = <MemoryConnection as Connection>::Recv;

        async fn open_uni(&self) -> Result<Self::Send, xmoq_moqt_io::ConnectionLost> {
            Ok(Ordering(self.0.open_uni().await?, self.1.clone()))
        }

        async fn open_bi(&self) -> Result<(Self::Send, Self::Recv), xmoq_moqt_io::ConnectionLost> {
            let (send, recv) = self.0.open_bi().await?;
            Ok((Ordering(send, self.1.clone()), recv))
        }

        async fn accept_uni(&self) -> Result<Self::Recv, xmoq_moqt_io::ConnectionLost> {
            self.0.accept_uni().await
        }

        async fn accept_bi(
            &self,
        ) -> Result<(Self::Send, Self::Recv), xmoq_moqt_io::ConnectionLost> {
            let (send, recv) = self.0.accept_bi().await?;
            Ok((Ordering(send, self.1.clone()), recv))
        }

        async fn read_datagram(&self) -> Result<Vec<u8>, xmoq_moqt_io::ConnectionLost> {
            self.0.read_datagram().await
        }

        fn close(&self, code: u32, reason: &[u8]) {
            self.0.close(code, reason);
        }

        async fn closed(&self) -> xmoq_moqt_io::ConnectionEnd {
            self.0.closed().await
        }
    }

    #[tokio::test]
    async fn an_order_to_stop_that_comes_between_two_writes_or_after_the_last_is_kept() {
        // A stream of a header and two pieces is three writes.
        // (the write after which the order comes; what the subscriber gets
        //  before the reset: the header and so many pieces)
        for (after, pieces_read) in [(1, 0), (2, 1), (3, 2)] {
            let (relay, subscriber) = pair();
            let (orders, ordered) = oneshot::channel();
            let hook = OrderAfter {
                after,
                written: Arc::default(),
                order: Arc::new(std::sync::Mutex::new(Some(orders))),
            };
            let connection = Arc::new(Orderly(relay, hook));
            let write = FetchWrite {
                connection: Arc::clone(&connection),
                header: FetchHeader { request_id: 4 },
                send_order: watch::channel(0).1,
                pieces: vec![indicator(0), indicator(1)],
                patience: IN_TIME,
                whole: WHOLE,
                trace: Trace::default(),
                label: "test".to_string(),
            };
            // The order is there when the write has completed: also after
            // the last one, where only the end of the stream is left.
            // The pieces that were written whole before the order came:
            // none after the header, and all of them after the last.
            let ended = in_time(write.run(ordered)).await;
            assert_eq!(
                (ended.how, ended.pieces),
                (WriteEnd::Aborted(0x12), pieces_read),
                "{after}"
            );
            let stream = in_time(subscriber.accept_uni()).await.unwrap();
            let (events, end) = in_time(read_all(stream, Duration::ZERO)).await;
            // What was written and not read may be gone with the reset.
            assert!(events.len() <= 1 + pieces_read, "{after}: {events:?}");
            assert_eq!(end, Some(ReadError::Reset(Some(0x12))), "{after}");
        }
    }

    #[tokio::test]
    async fn a_subscriber_that_stops_the_stream_or_is_gone_ends_it_as_cancelled() {
        // The subscriber stops the stream, with another code than the relay
        // answers with: CANCELLED (0x1).
        let (relay, subscriber) = pair_with_window(64);
        let (stopped, _session) = write(relay, (0..50).map(indicator).collect(), IN_TIME);
        let writing = tokio::spawn(stopped.run(oneshot::channel().1));
        in_time(subscriber.accept_uni()).await.unwrap().stop(0x77);
        assert_eq!(in_time(writing).await.unwrap().given_up(), Some(0x1));

        // The connection is gone before the stream is opened.
        let (relay, subscriber) = pair();
        subscriber.close(0, b"gone");
        let (unopened, _session) = write(relay, vec![indicator(0)], IN_TIME);
        assert_eq!(
            in_time(unopened.run(oneshot::channel().1)).await.given_up(),
            Some(0x1)
        );
    }

    #[tokio::test]
    async fn an_indicator_is_written_as_the_bytes_both_implementations_read() {
        // `fixtures/moqt-18-by-hand/fetch-stream-indicator-first.hex`: a
        // FETCH_HEADER with the Request ID 0, an End of Unknown Range
        // indicator at {3, 1}, and then an Object, which the relay does not
        // write. The file is read by the decoders of `crates/moqt` and of
        // `@xmoq/moqt`.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/moqt-18-by-hand/fetch-stream-indicator-first.hex"
        );
        let fixture: Vec<u8> = std::fs::read_to_string(path)
            .unwrap()
            .split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect();
        assert_eq!(fixture[..6], [0x05, 0x00, 0x81, 0x0c, 0x03, 0x01]);

        let (relay, subscriber) = pair();
        let connection = Arc::new(relay);
        let unknown = Location {
            group: 3,
            object: 1,
        };
        let write = FetchWrite {
            connection: Arc::clone(&connection),
            header: FetchHeader { request_id: 0 },
            send_order: watch::channel(0).1,
            pieces: vec![FetchPiece::EndOfRange(RangeEnd::Unknown, unknown)],
            patience: IN_TIME,
            whole: WHOLE,
            trace: Trace::default(),
            label: "test".to_string(),
        };
        assert_eq!(
            in_time(write.run(oneshot::channel().1)).await.given_up(),
            None
        );
        let mut stream = in_time(subscriber.accept_uni()).await.unwrap();
        let mut written = Vec::new();
        let mut buffer = [0u8; 64];
        while let Some(read) = in_time(stream.read(&mut buffer)).await.unwrap() {
            written.extend_from_slice(&buffer[..read]);
        }
        assert_eq!(written, fixture[..6]);
    }

    #[test]
    fn the_end_of_a_write_is_logged_with_what_was_written_and_the_code_of_the_reset() {
        // (how the writing ended; the pieces written whole; the line)
        let cases = [
            (
                WriteEnd::Finished,
                8,
                "FETCH_WRITE_ENDED session=2 stream=5 finished pieces=8",
            ),
            // TOO_FAR_BEHIND (0x5).
            (
                WriteEnd::NotTaken,
                0,
                "FETCH_WRITE_ENDED session=2 stream=5 not_taken pieces=0 code=0x5",
            ),
            // TOO_FAR_BEHIND (0x5), for a subscriber that stopped taking.
            (
                WriteEnd::TooSlow,
                2,
                "FETCH_WRITE_ENDED session=2 stream=5 too_slow pieces=2 code=0x5",
            ),
            // CANCELLED (0x1), for a subscriber that stopped the stream.
            (
                WriteEnd::Stopped,
                3,
                "FETCH_WRITE_ENDED session=2 stream=5 stopped pieces=3 code=0x1",
            ),
            // The code of the order: MALFORMED_TRACK (0x12).
            (
                WriteEnd::Aborted(0x12),
                3,
                "FETCH_WRITE_ENDED session=2 stream=5 aborted pieces=3 code=0x12",
            ),
            // INTERNAL_ERROR (0x0).
            (
                WriteEnd::Failed,
                1,
                "FETCH_WRITE_ENDED session=2 stream=5 failed pieces=1 code=0x0",
            ),
        ];
        for (how, pieces, line) in cases {
            let written = Written { how, pieces };
            assert_eq!(written.line(SessionKey(2), StreamKey(5)), line);
        }
    }

    #[tokio::test]
    async fn work_in_a_task_of_its_own_ends_when_the_task_that_waits_for_it_does() {
        // What the work holds, as a write holds its connection.
        let held = Arc::new(());
        let holds = Arc::clone(&held);
        let waiting = tokio::spawn(in_a_task_that_ends_with_this_call(async move {
            std::future::pending::<()>().await;
            drop(holds);
        }));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(Arc::strong_count(&held), 2);
        // The session goes: its tasks are aborted.
        waiting.abort();
        in_time(async {
            while Arc::strong_count(&held) > 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        // And work that came to its end is handed over.
        assert_eq!(
            in_a_task_that_ends_with_this_call(async { 7 }).await.ok(),
            Some(7)
        );
    }
}
