//! The `XmoQ` relay: `MoQ` Transport draft 18 over WebTransport.
//!
//! It takes PUBLISH from publishers and SUBSCRIBE from subscribers and forwards
//! the Objects of each track to its subscribers. A subscriber gets what
//! arrives after it subscribed, from the Start Location of its Subscription
//! Filter on; of what came before, the relay keeps the latest Group of each
//! track to answer a FETCH with (the cache of `xmoq-relay-core`), and asks
//! the publisher for the rest.
//!
//! Authorization is off unless the relay is given a key (`--auth-key-hex`,
//! the module `auth`): without one, anyone who can reach the port can
//! publish and subscribe.
//!
//! What is decided where: `xmoq-relay-core` decides who gets what and who is
//! cut off, `xmoq-moqt-io` reads and writes the streams, and this crate
//! connects the two.

mod auth;
mod hub;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;
use xmoq_room_token::{key_of_hex, unhex};

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::Semaphore;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt_io::StreamConfig;
use xmoq_moqt_io::wt::{self, Congestion, QuicLimits, WtConnection, WtServer, accept};

use crate::hub::{Arrival, GOAWAY_TIMEOUT, HubConfig, HubHandle, Log};

/// How long a client may take over the WebTransport handshake. Provisional.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Handshakes in progress at once. Further connection attempts wait to be
/// taken, in the queue of the QUIC endpoint. Provisional.
///
/// WHY: the limit on sessions only counts a connection once its handshake is
/// done, so without this one a client could keep any number of half-open
/// connections, each with a task, for the length of the timeout.
const MAX_HANDSHAKES: usize = 64;

/// How long the relay waits after its GOAWAY before it closes the sessions
/// that are left, where `--goaway-timeout-ms` does not say. Decided on
/// 2026-10-24: the 10 seconds `moqtail`'s relay waits (relay@0.15.0,
/// read on 2026-10-24).
const GOAWAY_TIMEOUT_MS: u64 = 10_000;

/// The flags, and what each is without it. Every number is more than 0
/// where the flag says it.
const USAGE: &str = "\
xmoq-relay --port <port> --cert-file <pem> --key-file <pem> [flags]
  --congestion cubic|newreno|bbr        the congestion control of its connections (cubic)
  --idle-timeout-ms <ms>                nothing from a peer for so long, and its connection is lost (6000)
  --setup-timeout-ms <ms>               how long a client has for its SETUP (10000)
  --idle-session-ms <ms>                how long a session past its SETUP may hold nothing (30000)
  --max-sessions <n>                    the sessions it takes at once (1000)
  --when-full <url>                     where a session that comes when it is full is sent: a GOAWAY
                                        after its SETUP names this URL with sent-away=1 in its query,
                                        and a session that came with that is closed; the relay there
                                        has to have the same certificate (closed with INTERNAL_ERROR)
  --subscription-send-bytes <n>, --subscription-send-objects <n>, --subscription-send-streams <n>
                                        what may wait to be written to one subscription before it
                                        ends with TOO_FAR_BEHIND (4 MiB, 4096, 1024)
  --session-send-bytes <n>, --session-send-objects <n>, --session-send-streams <n>
                                        what may wait to be written to one session, all its
                                        subscriptions together, before the one with the most ends
                                        (8 MiB, 16384, 4096)
  --relay-send-bytes <n>                what may wait to be written to all sessions together (2 GiB)
  --announce-at-once <n>                the PUBLISH_NAMESPACE requests a session may send at once (5)
  --announce-every-ms <ms>              the time in which a session gets one of them back (1000)
  --peer-requests-per-session <n>       the requests a session may have open at the relay (256)
  --own-requests-per-session <n>        the requests the relay may have open on a session: SUBSCRIBE,
                                        FETCH and TRACK_STATUS, not the namespaces it announces
                                        upstream (64)
  --namespaces-per-session <n>          the namespaces a session may have announced (64)
  --requests-for-one-session <n>        the requests the relay has open on one publisher for one
                                        subscriber's session (8)
  --fetches-for-one-session <n>         how many of them are FETCH requests (4)
  --cached-fetch-whole-ms <ms>          the least time a subscriber has for the whole of a fetch
                                        stream the relay writes from its cache (30000)
  --cached-fetch-least-kbps <n>         the least rate it has to take such a stream at; a larger
                                        Group has as long as it takes at that rate (500)
  --held-beyond-cache-bytes <n>         the bytes of Groups its cache has let go of that those fetch
                                        streams may keep alive; a session has an eighth (256 MiB)
  --auth-key-hex <hex>                  an experiment: the key tokens are checked with (no tokens)
  --auth-setup on|off                   with a key, whether a SETUP is asked for a token (on)
  --upstream <url>, --upstream-cert-sha256 <hex>
                                        the relay above this one, asked for the tracks nobody
                                        announced here; the two go together (none)
  --upstream-namespace <a/b>            the namespace asked of it (every namespace)
  --redirect-uri <uri>                  the New Session URI of the GOAWAY sent when the relay is told
                                        to stop (empty: the URI the client used)
  --goaway-timeout-ms <ms>              the Timeout of that GOAWAY, and the wait before the sessions
                                        that are left are closed (10000)
  --delivery-patience-ms <ms>           an experiment: what a subscriber gets that asked for no
                                        delivery timeout (none)
  --stats-every-ms <ms>                 for measuring: a STATS line for each session this often (none)
  --log-arrivals on|off                 for measuring: an ARRIVED line on stderr for every Object (off)
  --slow-open-ms <ms>                   a DATA_OPENED line for each subgroup stream towards a
                                        subscriber that took this long to open; 0 for every one (10)";

/// The QUIC limits that go with the limits of the relay. Provisional.
///
/// Three limits act on the requests a peer opens, and this is the one place
/// that sets how they relate:
///
/// 1. `MAX_PEER_REQUESTS` (256) of the session: a request beyond it is
///    answered with `EXCESSIVE_LOAD`.
/// 2. `max_request_streams` (264) of the stream handling: that many request
///    streams are read at once. It is above 1, so that the answer of 1 can be
///    given.
/// 3. The stream limit of QUIC, set to 2 here: a peer cannot open more
///    streams than are read, and waits in its own QUIC stack instead.
///
/// So 1 acts first, and a peer that ignores `EXCESSIVE_LOAD` is held at 3.
/// With quinn's default of 100 bidirectional streams, 3 would act first and
/// `EXCESSIVE_LOAD` would never be sent. The same goes for unidirectional
/// streams, where the stream handling reads 64 at once.
///
/// The windows bound what the QUIC layer holds for one peer; `HubConfig` adds
/// them up with the rest. The receive window of the connection is what the
/// peer's unidirectional streams can hold, filled to their window and not
/// read, and 1 MiB beyond it for the request streams
/// (`QuicLimits::leaves_room_for_control_streams`, §11.4.2): 67 streams of
/// 128 KiB and 1 MiB, 9.375 MiB with the 64 streams read at once.
///
/// WHY 128 KiB a stream, provisional: one stream carries at most its window
/// in a round trip, and a Group of video goes on one stream; 128 KiB is
/// about 10 Mbit/s at 100 ms, room for the demo's 2.65 Mbit/s and for a
/// 720p video of 2.5 Mbit/s. WHY NOT 48 KiB with the connection's 4 MiB of
/// before: about 3.9 Mbit/s at 100 ms leaves no room for those. WHY NOT the
/// 256 KiB of before: the connection would need about 18 MiB, some 18 GiB
/// for the 1,000 sessions a relay takes by default (`--max-sessions`);
/// with 128 KiB the worst is about 9.2 GiB (9.375 MiB a session).
fn quic_limits(
    streams: &StreamConfig,
    idle_timeout: Duration,
    congestion: Congestion,
) -> QuicLimits {
    const STREAM_RECEIVE_WINDOW: u32 = 128 * 1024;
    const SEND_WINDOW: u32 = 4 * 1024 * 1024;
    let (max_uni_streams, max_bi_streams) = xmoq_moqt_io::wt::streams_read_at_once(streams);
    QuicLimits {
        max_uni_streams,
        max_bi_streams,
        stream_receive_window: STREAM_RECEIVE_WINDOW,
        receive_window: xmoq_moqt_io::wt::receive_window_for(
            max_uni_streams,
            STREAM_RECEIVE_WINDOW,
        ),
        send_window: SEND_WINDOW,
        idle_timeout,
        congestion,
    }
}

struct Args {
    port: u16,
    cert_file: String,
    key_file: String,
    /// How long nothing may arrive from a peer before its connection counts
    /// as lost. A run that waits for the relay to notice a vanished peer
    /// sets it lower than the default.
    idle_timeout: Duration,
    /// `--congestion cubic|newreno|bbr`: the congestion control of the
    /// relay's connections ([`Congestion`]). For measuring.
    ///
    /// WHY Cubic unless told (decided on 2026-10-12): where the link's rate
    /// is the limit and its queue drops, Cubic was as good or better, and BBR
    /// sent more than the link takes
    /// (docs/experiments/2026-10-12-cubic-and-bbr-over-a-link-with-too-little-room.md).
    /// WHY NOT BBR: it was better only where packets are lost at random
    /// (docs/experiments/2026-10-12-video-over-a-link-that-delays-and-loses.md),
    /// which of the two a viewer's path is like is not known, and
    /// `quinn-proto` 0.11.19 calls its BBR experimental. To be looked at
    /// again with runs over real links.
    congestion: Congestion,
    /// `--max-sessions`: how many sessions the relay takes at once, where it
    /// is not the relay's own number ([`HubConfig::max_sessions`]). More
    /// than 0: there is no way to have none.
    max_sessions: Option<usize>,
    /// `--when-full`: where a session goes that comes when the relay is
    /// full ([`HubConfig::when_full`]). Closed where it is not said.
    when_full: Option<String>,
    /// `--slow-open-ms`: [`HubConfig::slow_open`], for a run that looks at
    /// how long streams take to open; 0 logs every stream.
    slow_open: Option<Duration>,
    /// `--setup-timeout-ms`: [`HubConfig::setup_timeout`], how long a
    /// client has for its SETUP.
    setup_timeout: Option<Duration>,
    /// `--idle-session-ms`: [`HubConfig::idle_session_timeout`], how long
    /// a session that has sent its SETUP may hold nothing.
    idle_session: Option<Duration>,
    /// `--announce-every-ms`: [`HubConfig::announcement_every_ms`], the time
    /// in which a session may send one `PUBLISH_NAMESPACE`.
    announce_every: Option<u64>,
    /// `--announce-at-once`: [`HubConfig::announcements_at_once`], how many
    /// it may send at once.
    announce_at_once: Option<u32>,
    /// `--requests-for-one-session`, `--fetches-for-one-session`:
    /// [`HubConfig::requests_for_one_subscriber`] and
    /// [`HubConfig::fetches_for_one_subscriber`].
    for_one_session: (Option<usize>, Option<usize>),
    /// `--cached-fetch-whole-ms`, `--cached-fetch-least-kbps`:
    /// [`HubConfig::cached_fetch_whole_ms`] and
    /// [`HubConfig::cached_fetch_least_kbps`].
    cached_fetch: (Option<u64>, Option<u64>),
    /// `--held-beyond-cache-bytes`:
    /// [`HubConfig::held_beyond_cache_bytes`].
    held_beyond_cache: Option<usize>,
    /// `--delivery-patience-ms`: [`xmoq_moqt_io::SendLimits::delivery_patience`].
    /// An experiment: what a subscriber gets that set no
    /// `SUBGROUP_DELIVERY_TIMEOUT`, and the most one that set a longer one
    /// gets. Without it only a subscriber's own timeout gives a stream up.
    delivery_patience: Option<Duration>,
    /// The limits of what waits to be written ([`SendFlags`]).
    send: SendFlags,
    /// What one session may have ([`SessionFlags`]).
    session: SessionFlags,
    /// `--stats-every-ms`: [`HubConfig::stats_every`]. For measuring.
    stats_every: Option<Duration>,
    /// `--log-arrivals on`: [`HubConfig::log_arrivals`]. For measuring.
    log_arrivals: bool,
    /// `--auth-key-hex`: the key requests are authorized with
    /// ([`HubConfig::auth_key`]). An experiment.
    auth_key: Option<Vec<u8>>,
    /// `--auth-setup off`: with a key, the SETUP of a client is not asked
    /// for a token ([`HubConfig::auth_setup`]). On with a key otherwise.
    auth_setup: bool,
    /// `--redirect-uri`: the New Session URI of the GOAWAY the relay sends
    /// when it is told to stop; empty where it is not said, for the URI the
    /// client used (§10.4). The name is `moqtail`'s relay's.
    redirect_uri: Vec<u8>,
    /// `--goaway-timeout-ms`: the Timeout of that GOAWAY, and how long the
    /// relay waits before it closes the sessions that are left
    /// ([`GOAWAY_TIMEOUT_MS`] where it is not said).
    goaway_timeout_ms: u64,
    /// `--upstream`, `--upstream-cert-sha256`, `--upstream-namespace`: the
    /// relay above this one, which is asked for the tracks of that
    /// namespace nobody announced here ([`hub::Upstream`]).
    upstream: Option<UpstreamArgs>,
}

/// `--session-send-bytes`, `--session-send-objects`,
/// `--session-send-streams` ([`HubConfig::send`]);
/// `--subscription-send-bytes`, `--subscription-send-objects`,
/// `--subscription-send-streams` ([`HubConfig::send_for_one_subscription`]);
/// `--relay-send-bytes` ([`HubConfig::send_for_the_relay_bytes`]). Each is
/// more than 0 where it is said: none of the limits can be had away with.
#[derive(Default)]
struct SendFlags {
    session: [Option<usize>; 3],
    subscription: [Option<usize>; 3],
    relay_bytes: Option<usize>,
}

impl SendFlags {
    /// Takes `flag` where it is one of these: true then.
    fn take(&mut self, flag: &str, value: &str) -> Result<bool> {
        let place = match flag {
            "--session-send-bytes" => &mut self.session[0],
            "--session-send-objects" => &mut self.session[1],
            "--session-send-streams" => &mut self.session[2],
            "--subscription-send-bytes" => &mut self.subscription[0],
            "--subscription-send-objects" => &mut self.subscription[1],
            "--subscription-send-streams" => &mut self.subscription[2],
            "--relay-send-bytes" => &mut self.relay_bytes,
            _ => return Ok(false),
        };
        *place = Some(more_than_none(flag, value)?);
        Ok(true)
    }

    fn apply(&self, config: &mut HubConfig) {
        let set = |limits: &mut xmoq_moqt_io::SendLimits,
                   [bytes, objects, streams]: [Option<usize>; 3]| {
            limits.bytes = bytes.unwrap_or(limits.bytes);
            limits.objects = objects.unwrap_or(limits.objects);
            limits.streams = streams.unwrap_or(limits.streams);
        };
        set(&mut config.send, self.session);
        set(&mut config.send_for_one_subscription, self.subscription);
        if let Some(bytes) = self.relay_bytes {
            config.send_for_the_relay_bytes = bytes;
        }
    }
}

/// `--peer-requests-per-session`, `--own-requests-per-session`,
/// `--namespaces-per-session` ([`HubConfig::limits_of_a_session`]). Each is
/// more than 0 where it is said.
#[derive(Default)]
struct SessionFlags {
    peer_requests: Option<usize>,
    own_requests: Option<usize>,
    namespaces: Option<usize>,
}

impl SessionFlags {
    /// Takes `flag` where it is one of these: true then.
    fn take(&mut self, flag: &str, value: &str) -> Result<bool> {
        let place = match flag {
            "--peer-requests-per-session" => &mut self.peer_requests,
            "--own-requests-per-session" => &mut self.own_requests,
            "--namespaces-per-session" => &mut self.namespaces,
            _ => return Ok(false),
        };
        *place = Some(more_than_none(flag, value)?);
        Ok(true)
    }

    fn apply(&self, config: &mut HubConfig) {
        let limits = &mut config.limits_of_a_session;
        limits.peer_requests = self.peer_requests.unwrap_or(limits.peer_requests);
        limits.own_requests = self.own_requests.unwrap_or(limits.own_requests);
        limits.namespaces = self.namespaces.unwrap_or(limits.namespaces);
        config.streams.max_request_streams = hub::request_streams_for(limits.peer_requests);
    }
}

struct UpstreamArgs {
    url: String,
    /// The SHA-256 of its certificate, which is pinned.
    cert_sha256: [u8; 32],
    /// Every namespace where `--upstream-namespace` is not said; its fields
    /// are separated by `/`.
    namespace: TrackNamespace,
}

/// The upstream flags, all or none of the first two.
fn upstream_args(
    url: Option<String>,
    cert_sha256: Option<String>,
    namespace: Option<String>,
) -> Result<Option<UpstreamArgs>> {
    let (url, cert_sha256) = match (url, cert_sha256) {
        (None, None) if namespace.is_none() => return Ok(None),
        (Some(url), Some(cert_sha256)) => (url, cert_sha256),
        _ => bail!("--upstream and --upstream-cert-sha256 go together"),
    };
    let cert_sha256 = unhex(&cert_sha256)
        .context("--upstream-cert-sha256 is not hex")?
        .try_into()
        .map_err(|_| anyhow!("--upstream-cert-sha256 must be 32 bytes"))?;
    let fields = namespace
        .unwrap_or_default()
        .split('/')
        .filter(|field| !field.is_empty())
        .map(|field| field.as_bytes().to_vec())
        .collect();
    let namespace = TrackNamespace::new(fields).context("--upstream-namespace")?;
    Ok(Some(UpstreamArgs {
        url,
        cert_sha256,
        namespace,
    }))
}

/// The number a flag gives.
fn small(flag: &str, value: &str) -> Result<u32> {
    value.parse().with_context(|| flag.to_string())
}

/// The number a flag gives.
fn number(flag: &str, value: &str) -> Result<u64> {
    value.parse().with_context(|| flag.to_string())
}

/// The number a flag gives.
fn count(flag: &str, value: &str) -> Result<usize> {
    value.parse().with_context(|| flag.to_string())
}

/// A count that may not be 0: a limit that has no way to be none.
fn more_than_none(flag: &str, value: &str) -> Result<usize> {
    let count = count(flag, value)?;
    if count == 0 {
        bail!("{flag} must be more than 0");
    }
    Ok(count)
}

/// The key a flag gives (`key_of_hex`).
fn key(flag: &str, value: &str) -> Result<Vec<u8>> {
    key_of_hex(value).with_context(|| format!("{flag} is hex of one byte at least"))
}

/// What a flag of `on` or `off` gives.
fn on_or_off(flag: &str, value: &str) -> Result<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        other => bail!("{flag} {other}: on or off"),
    }
}

/// The milliseconds a flag gives.
fn millis(flag: &str, value: &str) -> Result<Duration> {
    Ok(Duration::from_millis(
        value.parse().with_context(|| flag.to_string())?,
    ))
}

fn parse_args() -> Result<Args> {
    let mut port = None;
    let mut cert_file = None;
    let mut key_file = None;
    let mut idle_timeout = xmoq_moqt_io::wt::IDLE_TIMEOUT;
    let mut congestion = Congestion::default();
    let (mut max_sessions, mut when_full) = (None, None);
    let (mut slow_open, mut setup_timeout, mut idle_session) = (None, None, None);
    let (mut announce_every, mut announce_at_once) = (None, None);
    let mut for_one_session = (None, None);
    let mut cached_fetch = (None, None);
    let mut held_beyond_cache = None;
    let mut delivery_patience = None;
    let mut stats_every = None;
    let mut send = SendFlags::default();
    let mut session = SessionFlags::default();
    let mut log_arrivals = false;
    let mut auth_key = None;
    let mut auth_setup = true;
    let mut redirect_uri = Vec::new();
    let mut goaway_timeout_ms = GOAWAY_TIMEOUT_MS;
    let (mut upstream, mut upstream_cert, mut upstream_namespace) = (None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| anyhow!("missing value for {flag}"))?;
        match flag.as_str() {
            "--port" => port = Some(value.parse().context("--port")?),
            "--cert-file" => cert_file = Some(value),
            "--key-file" => key_file = Some(value),
            "--congestion" => {
                congestion = match value.as_str() {
                    "cubic" => Congestion::Cubic,
                    "newreno" => Congestion::NewReno,
                    "bbr" => Congestion::Bbr,
                    other => bail!("--congestion {other}: cubic, newreno or bbr"),
                };
            }
            "--idle-timeout-ms" => {
                let millis: u64 = value.parse().context("--idle-timeout-ms")?;
                if millis == 0 {
                    bail!("--idle-timeout-ms must be more than 0");
                }
                idle_timeout = Duration::from_millis(millis);
            }
            "--max-sessions" => max_sessions = Some(more_than_none(&flag, &value)?),
            "--when-full" => when_full = Some(value),
            "--slow-open-ms" => slow_open = Some(millis(&flag, &value)?),
            "--setup-timeout-ms" => setup_timeout = Some(millis(&flag, &value)?),
            "--announce-every-ms" => announce_every = Some(number(&flag, &value)?),
            "--held-beyond-cache-bytes" => held_beyond_cache = Some(count(&flag, &value)?),
            "--cached-fetch-whole-ms" => cached_fetch.0 = Some(number(&flag, &value)?),
            "--cached-fetch-least-kbps" => cached_fetch.1 = Some(number(&flag, &value)?),
            "--requests-for-one-session" => for_one_session.0 = Some(count(&flag, &value)?),
            "--fetches-for-one-session" => for_one_session.1 = Some(count(&flag, &value)?),
            "--announce-at-once" => announce_at_once = Some(small(&flag, &value)?),
            "--idle-session-ms" => idle_session = Some(millis(&flag, &value)?),
            "--delivery-patience-ms" => delivery_patience = Some(millis(&flag, &value)?),
            "--stats-every-ms" => stats_every = Some(millis(&flag, &value)?),
            "--log-arrivals" => log_arrivals = on_or_off(&flag, &value)?,
            "--auth-key-hex" => auth_key = Some(key(&flag, &value)?),
            "--auth-setup" => auth_setup = on_or_off(&flag, &value)?,
            "--redirect-uri" => redirect_uri = value.into_bytes(),
            "--goaway-timeout-ms" => goaway_timeout_ms = number(&flag, &value)?,
            "--upstream" => upstream = Some(value),
            "--upstream-cert-sha256" => upstream_cert = Some(value),
            "--upstream-namespace" => upstream_namespace = Some(value),
            other if send.take(other, &value)? || session.take(other, &value)? => {}
            other => bail!("unknown flag {other}"),
        }
    }
    Ok(Args {
        port: port.context("--port is required")?,
        cert_file: cert_file.context("--cert-file is required")?,
        key_file: key_file.context("--key-file is required")?,
        idle_timeout,
        congestion,
        max_sessions,
        when_full,
        slow_open,
        setup_timeout,
        idle_session,
        announce_every,
        announce_at_once,
        for_one_session,
        cached_fetch,
        held_beyond_cache,
        delivery_patience,
        send,
        session,
        stats_every,
        log_arrivals,
        auth_key,
        auth_setup,
        redirect_uri,
        goaway_timeout_ms,
        upstream: upstream_args(upstream, upstream_cert, upstream_namespace)?,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error:#}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let log = Log::stdout();
    let mut config = HubConfig {
        log: Some(log.clone()),
        ..HubConfig::default()
    };
    if let Some(max_sessions) = args.max_sessions {
        config.max_sessions = max_sessions;
    }
    config.when_full = args.when_full;
    if let Some(slow_open) = args.slow_open {
        config.slow_open = slow_open;
    }
    if let Some(setup_timeout) = args.setup_timeout {
        config.setup_timeout = setup_timeout;
    }
    if let Some(idle_session) = args.idle_session {
        config.idle_session_timeout = idle_session;
    }
    config.announcement_every_ms = args.announce_every;
    config.announcements_at_once = args.announce_at_once;
    (
        config.requests_for_one_subscriber,
        config.fetches_for_one_subscriber,
    ) = args.for_one_session;
    config.held_beyond_cache_bytes = args.held_beyond_cache;
    if let Some(whole_ms) = args.cached_fetch.0 {
        config.cached_fetch_whole_ms = whole_ms;
    }
    if let Some(least_kbps) = args.cached_fetch.1 {
        config.cached_fetch_least_kbps = least_kbps;
    }
    config.send.delivery_patience = args.delivery_patience;
    args.send.apply(&mut config);
    args.session.apply(&mut config);
    config.stats_every = args.stats_every;
    config.log_arrivals = args.log_arrivals;
    config.auth_key = args.auth_key;
    config.auth_setup = args.auth_setup;
    let limits = quic_limits(&config.streams, args.idle_timeout, args.congestion);
    let server = WtServer::bind(args.port, &args.cert_file, &args.key_file, limits)?;
    println!("LISTENING {} port={}", config.implementation, args.port);
    let upstream = args.upstream.map(|upstream| {
        let streams = config.streams.clone();
        let (url, cert_sha256) = (Arc::new(upstream.url), upstream.cert_sha256);
        let connect: hub::Connect<WtConnection> = Arc::new(move |moved_to: Option<String>| {
            let (url, streams) = (Arc::clone(&url), streams.clone());
            Box::pin(async move {
                // WHY the same certificate hash at the URI of a GOAWAY: the
                // pin is the one way this relay checks a peer, and nothing
                // gives it another (known gap).
                let url = moved_to.as_deref().unwrap_or(&url);
                wt::connect(url, cert_sha256, &streams)
                    .await
                    .map_err(|error| error.to_string())
            })
        });
        hub::Upstream {
            namespace: upstream.namespace,
            connect,
        }
    });
    let (hub, mut hub_task) = hub::spawn_with_upstream(config, upstream);
    tokio::select! {
        () = accept_sessions(&server, &hub, &log) => bail!("the server stopped accepting"),
        () = told_to_stop() => {}
        // WHY end the process: the hub is the one task that serves every
        // session. Without it the relay would go on accepting connections that
        // nobody answers.
        ended = &mut hub_task => bail!("the relay's hub ended: {ended:?}"),
    }
    // §3.6: GOAWAY to every session, and the sessions that are left are
    // closed with GOAWAY_TIMEOUT when its Timeout is over. WHY a connection
    // that comes meanwhile is refused, and not taken and told: a client sent
    // to the URI it used would come back to this relay at once, again and
    // again, until it ends; refused, it waits and tries again, and finds the
    // relay that takes this one's place.
    let timeout = Duration::from_millis(args.goaway_timeout_ms);
    tokio::select! {
        () = refuse_sessions(&server) => {}
        _ = tokio::time::timeout(timeout, hub.go_away(args.redirect_uri, args.goaway_timeout_ms)) => {}
        _ = &mut hub_task => return Ok(()),
    }
    hub.stop_with(GOAWAY_TIMEOUT, "the GOAWAY's time is over")
        .await;
    Ok(())
}

/// Refuses every connection attempt, for a relay that has sent its GOAWAY.
async fn refuse_sessions(server: &WtServer) {
    while let Some(incoming) = server.incoming().await {
        incoming.refuse();
    }
}

/// Completes when the process is told to end: SIGINT, and SIGTERM where
/// there is one.
async fn told_to_stop() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        // Without the handler the default holds: the process ends at once.
        let Ok(mut terminate) = signal(SignalKind::terminate()) else {
            return std::future::pending().await;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn accept_sessions(server: &WtServer, hub: &HubHandle<WtConnection>, log: &Log) {
    let handshakes = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    loop {
        let Ok(permit) = Arc::clone(&handshakes).acquire_owned().await else {
            return;
        };
        let Some(incoming) = server.incoming().await else {
            return;
        };
        let hub = hub.clone();
        let log = log.clone();
        // Each handshake runs on its own, so that a slow client does not hold
        // up the others.
        tokio::spawn(async move {
            let accepted = tokio::time::timeout(HANDSHAKE_TIMEOUT, accept(incoming)).await;
            drop(permit);
            match accepted {
                Ok(Ok((connection, asked))) => {
                    log.print(format!(
                        "SESSION_ACCEPTED authority={} path={}",
                        asked.authority, asked.path
                    ));
                    let arrival = Arrival::of_query(asked.query.as_deref());
                    hub.add_session(connection, arrival).await;
                }
                Ok(Err(error)) => log.print(format!("HANDSHAKE_FAILED {error}")),
                Err(_) => log.print("HANDSHAKE_FAILED timed out".to_string()),
            }
        });
    }
}

#[cfg(test)]
mod usage {
    use std::collections::BTreeSet;

    /// Every flag the relay reads is in `USAGE`, the one place the flags
    /// are written down.
    #[test]
    fn names_every_flag_the_relay_reads() {
        let flags: BTreeSet<&str> = include_str!("main.rs")
            .split('"')
            .filter(|text| {
                text.len() > 2
                    && text.starts_with("--")
                    && text[2..].bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
            .collect();
        for flag in &flags {
            assert!(super::USAGE.contains(flag), "{flag} is not in USAGE");
        }
        assert_eq!(flags.len(), 37);
    }
}
