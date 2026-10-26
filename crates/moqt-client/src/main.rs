//! A `MoQ` Transport draft-18 client over WebTransport, built on `xmoq-moqt`.
//!
//! It subscribes to one track and prints the Objects it receives, or publishes
//! one Object. What it prints to stdout is a contract with the scripts that
//! drive it; see `report`.

mod auth;
mod catalog;
mod fetching;
mod hex;
mod io;
mod published;
mod report;
mod video;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::Instant;
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::fetch::{FetchHeader, FetchObject};
use xmoq_moqt::immutable::TrackRefusal;
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, Fetch, FetchRange, GoAway, ReasonPhrase, RequestError, publish_done_status,
    request_error_code, retry_interval, track_property,
};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter, SubscriptionFilter};
use xmoq_moqt::session::{
    ControlEvent, FetchStreamOpened, MalformedRequest, PeerRequest, PeerRequestEvent,
    PeerRequestId, PeerRequestOpened, RequestEvent, RequestId, RequestState, Role, Session,
    SubscribedAlias, Transport,
};

use xmoq_moqt::{ErrorScope, stream_reset_code};
use xmoq_moqt_io::refuse::{on_refused_closed, on_refused_message};
use xmoq_moqt_io::wt::{self, WtConnection};
use xmoq_moqt_io::{
    Connection, ConnectionEnd, Direction, IdleSubgroupStreamTimeout, PeerRequestStreamEvent,
    RequestClose, RequestStreamEvent, SendStream, StreamConfig, StreamEnd, StreamStops, Trace,
    UniEvent, client,
};

use crate::fetching::{BodyEnd, Fetches, Happened, Note, Reading, StreamEnded, Tell};
use crate::io::{DataInbox, RequestStream};
use crate::published::Published;
use xmoq_publisher::answers::{
    self, Answer, Answers, Bounds, Conclusion, MAX_ANSWERED_AT_ONCE, Pace, SAY_QUEUE_OF_A_PAGE,
};

/// Track Alias this client announces in PUBLISH. It publishes one track per
/// session, so any value is unique (§11.1).
const PUBLISH_TRACK_ALIAS: u64 = 1;

/// The Stream Count of a `PUBLISH_DONE` whose sender cannot give the number
/// (§10.11).
const STREAM_COUNT_UNKNOWN: u64 = (1 << 62) - 1;

/// How long the peer gets to acknowledge the close before the process exits.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// The bytes of payload the answers to the peer's FETCH requests being
/// written hold together ([`Bounds::bytes`]), as the gatherer and the
/// transcriber hold theirs; a FETCH whose answer does not fit is refused
/// with `EXCESSIVE_LOAD`. **Provisional.** WHY: each answer is a copy of
/// what the client keeps, up to `MAX_KEPT_BYTES` (4 MiB), and
/// [`MAX_ANSWERED_AT_ONCE`] of them would be 128 MiB. WHY 32 MiB: eight
/// answers of all the client keeps.
const MAX_BYTES_HELD: usize = 32 * 1024 * 1024;

/// How long the peer gets to take what was written to a stream before this
/// client goes on, which in the end means closing the session. Provisional.
///
/// WHY wait at all: closing the session drops what the transport has not
/// delivered yet, and this client closes right after its last write.
const DELIVERY_WAIT: Duration = Duration::from_secs(5);

/// How long a subgroup stream may take to be written, while the peer takes
/// nothing of it; then it is reset with `CANCELLED` and the client goes on.
/// The gatherer and the transcriber give theirs as long (`GROUP_WRITE_WAIT`
/// of `xmoq-publisher`). **Provisional.** WHY a limit: the
/// `PUBLISH_DONE` of a subscription waits for its streams to be closed
/// (§10.11), and a peer that takes nothing would hold it for good.
const SUBGROUP_WRITE_WAIT: Duration = Duration::from_secs(5);

/// How long the subscriber gets to end its direction of the request stream
/// after this client's `PUBLISH_DONE`, before the session is closed without
/// knowing whether everything arrived. Provisional.
///
/// WHY short: a peer that follows only the draft may never end it, and every
/// run against such a peer waits this long.
/// WHY longer than the 2 seconds an `XmoQ` relay waits for the counted
/// streams: the relay's reset at the end of its wait is to be seen, not cut
/// off by this client's own limit.
const PUBLISH_DONE_ANSWER_WAIT: Duration = Duration::from_secs(3);

/// How long the last Objects of a subscription are waited for after
/// `PUBLISH_DONE`, when fewer data streams ended than it counted (§10.11).
///
/// Provisional: the draft asks for a timer and names no duration, and `XmoQ` has
/// not decided one.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

enum Mode {
    /// Prints this many Objects, then leaves.
    Subscribe { objects: u64 },
    /// Sends Standalone FETCH requests for this range of the track, prints
    /// what comes on their fetch streams, and leaves when all are over.
    Fetch { start: Location, end: Location },
    /// Publishes one Object carrying this payload.
    Publish { payload: Vec<u8> },
    /// Announces the namespace with `PUBLISH_NAMESPACE` and sends no PUBLISH:
    /// the track is published to whoever sends SUBSCRIBE for it, which is how
    /// a relay asks for a track of an announced namespace (§9.5).
    PublishNamespace { payload: Vec<u8> },
    /// Asks with `SUBSCRIBE_NAMESPACE` for the namespaces under `--namespace`
    /// (§6.1, §10.18) and prints each NAMESPACE and `NAMESPACE_DONE`, until
    /// stdin ends.
    SubscribeNamespace,
    /// Sends one `TRACK_STATUS` for the track (§10.14), prints its answer
    /// (`TRACK_STATUS_OK ...` or `TRACK_STATUS_REFUSED ...`), and leaves.
    TrackStatus,
}

struct Args {
    mode: Mode,
    url: String,
    cert_sha256: [u8; 32],
    namespace: String,
    track_name: String,
    /// `--annex-b-out`, `--viewers`, `--video-in` and `--stay-after-ms`.
    video: video::VideoFlags,
    /// `--auth-token-hex`.
    auth: auth::AuthFlag,
    /// `--catalog-video`: `--track` names the catalog track of MSF, and
    /// `--mode subscribe` subscribes to the first H.264 video track of the
    /// catalog it reads there.
    catalog_video: bool,
    timeout_secs: u64,
    /// `--answer-wait-ms`, `--setup-wait-ms`, `--open-wait-ms`,
    /// `--rendezvous-timeout-ms` and `--idle-subgroup-stream-ms`.
    waits: Waits,
    /// `--new-group-request`, `--dynamic-groups`.
    new_groups: NewGroups,
    /// `--update-subgroup-timeout-ms`: `--mode subscribe` sends a
    /// `REQUEST_UPDATE` with this `SUBGROUP_DELIVERY_TIMEOUT` once its
    /// SUBSCRIBE is accepted (§10.9), and prints `UPDATE_ACCEPTED` or
    /// `UPDATE_FAILED` when the answer comes.
    update_subgroup_timeout_ms: Option<u64>,
    /// How long the publisher waits between the answer to its
    /// `PUBLISH_NAMESPACE` and its PUBLISH.
    publish_delay: Duration,
    /// How the Subgroup Headers this client sends give the Subgroup ID
    /// (§11.4.2): not at all (it is 0), as the ID of the first Object, or in
    /// the header. Every stream it sends is one Object, Object 0, so the
    /// Subgroup ID is 0 all three ways; what differs is when a receiver
    /// knows it.
    subgroup_id: SubgroupId,
    /// The Group ID of the first Object this client sends; each Object after
    /// it is the next Group. WHY a flag: two publishers of one track that
    /// both count from 0 send the same Groups, and a relay takes each from
    /// one of them. To see a track go on with the publisher that is left,
    /// the second has to start where the first stopped.
    first_group: u64,
    /// The Publisher Priority in the header of every subgroup stream this
    /// client sends; `None`, from `--publisher-priority default`, leaves it
    /// out, so that the Subgroup has the default of the subscription
    /// (§11.4.2). WHY a flag: a relay that takes a track from two publishers
    /// has to find a Subgroup that they send with different priorities
    /// (§2.4.2).
    publisher_priority: Option<u8>,
    /// The Object Properties of every Object this client sends (§11.2.1.2),
    /// from `--object-property`. Empty: the streams do not announce any.
    object_properties: Vec<KeyValuePair>,
    /// How long a SUBSCRIBE of the peer that this client serves waits for its
    /// `SUBSCRIBE_OK`. WHY a flag: a relay that asked this publisher for a
    /// track takes a PUBLISH for it from another publisher meanwhile, and on
    /// loopback the answer is there before anyone else can publish.
    serve_delay: Duration,
    /// How long a request of the peer waits for its refusal.
    ///
    /// WHY the two delays: on a real link, a SUBSCRIBE the relay sends after
    /// it accepted the namespace is answered a round trip later, and the
    /// PUBLISH is already under way by then (§5.1 has the SUBSCRIBE give way).
    /// On loopback that window is too short to hit on purpose; the delays
    /// widen it.
    answer_delay: Duration,
    /// `--subscription-filter`: the Subscription Filter of the SUBSCRIBE
    /// this client sends (§5.1.2); `None` for an unfiltered one. Largest
    /// Object without the flag.
    subscription_filter: Option<SubscriptionFilter>,
    /// What this client does only when the driver writes a line.
    on_line: OnLine,
    /// What the flags say of the FETCH requests this client sends.
    fetch: FetchArgs,
    trace: Trace,
}

/// The steps a client takes only when the driver writes a line on stdin.
///
/// WHY a line and not only a delay: a driver that wants a request to arrive
/// within a window some other peer opens (a relay that waits a few seconds
/// for an answer, or for the late streams of a track that ended) cannot have
/// this client start and connect inside that window on a loaded machine.
/// Everything that takes time is done before the line.
#[derive(Default)]
struct OnLine {
    /// `--publish-on-line`: the publisher prints `READY_TO_PUBLISH` where it
    /// would send its PUBLISH, and sends it when the driver writes the line
    /// `publish`.
    publish: bool,
    /// `--serve-on-line`: the client prints `ASKED request_id=N` for the
    /// SUBSCRIBE it would serve, and sends `SUBSCRIBE_OK` when the driver
    /// writes the line `serve`.
    serve: bool,
    /// `--subscribe-on-line`: the subscriber prints `READY_TO_SUBSCRIBE`
    /// once its session is set up, and sends its SUBSCRIBE when the driver
    /// writes the line `subscribe`.
    subscribe: bool,
}

/// The lines of the driver, on stdin.
type DriverLines = tokio::io::Lines<BufReader<tokio::io::Stdin>>;

/// Waits for the line `expected` of the driver.
///
/// # Errors
///
/// Fails for another line, and when stdin is closed: the driver is gone, and
/// the client leaves as it does wherever it waits for a line.
async fn wait_for_line(stdin: &mut DriverLines, expected: &str) -> Result<()> {
    match stdin.next_line().await? {
        Some(line) if line.trim() == expected => Ok(()),
        Some(line) => bail!("waited for the line {expected}, not {line:?}"),
        None => bail!("stdin closed before the line {expected}"),
    }
}

/// What the flags say of the FETCH requests this client sends.
struct FetchArgs {
    /// How many FETCH requests `--mode fetch` sends, all for the same range.
    /// WHY a flag: a relay asks one publisher for only so many at once.
    count: u64,
    /// The `FILL_TIMEOUT` of the FETCH requests this client sends (§10.2.5), in
    /// milliseconds; none without the flag.
    fill_timeout_ms: Option<u64>,
    /// Their `GROUP_ORDER` (§10.2.8); none without the flag, which asks for
    /// Ascending.
    group_order: Option<GroupOrder>,
    /// `--join`: the subscriber also sends a Joining FETCH that names its
    /// SUBSCRIBE (§10.12.2).
    join: Option<Join>,
    /// How this client reads a fetch stream.
    reading: Reading,
    /// `--fetch-stay-ms`: how long `--mode fetch` stays after every FETCH
    /// is over, with its streams as they are. WHY a flag: a client that
    /// leaves closes its connection, and what a publisher does about a
    /// FETCH that was cancelled is seen only on a connection that lasts.
    ///
    /// The stay does not watch the connection: the client stays the whole
    /// time and leaves with the exit code 0 also when the connection went
    /// meanwhile. It counts against `--timeout-secs`, so a stay beyond that
    /// ends as a run that took too long.
    stay: Duration,
    /// `--answer-fetch`: a publisher answers a Standalone FETCH for its
    /// track from the Objects it sent, at this pace. Without the flag it
    /// refuses every FETCH with `NOT_SUPPORTED`.
    ///
    /// WHY a flag and not what a publisher does: drivers count on a
    /// publisher that refuses, to see what a relay makes of the refusal.
    answering: Option<Pace>,
}

/// The Joining FETCH of `--join`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Join {
    /// Relative (`relative:N`) or Absolute (`absolute:N`).
    relative: bool,
    /// The Joining Start.
    start: u64,
    /// `--join-at-once`: the FETCH is sent right after the SUBSCRIBE, before
    /// its `SUBSCRIBE_OK`. WHY not always: the two travel on streams of
    /// their own, and a publisher that reads the FETCH first knows no
    /// subscription for it. Without the flag the FETCH waits for the
    /// `SUBSCRIBE_OK`.
    at_once: bool,
}

/// The most FETCH requests `--fetch-count` sends. WHY a limit: the client
/// sends them all before it reads an answer, on streams of which the peer
/// lets it have only so many open: 64 with an `XmoQ` relay, which lets a peer
/// open as many request streams as it reads at once
/// (`StreamConfig::max_request_streams`).
const MAX_FETCH_COUNT: u64 = 32;

/// The flags about FETCH, as they were read.
#[derive(Debug, Default, PartialEq, Eq)]
struct FetchFlags {
    start: Option<Location>,
    end: Option<Location>,
    count: Option<u64>,
    fill_timeout_ms: Option<u64>,
    group_order: Option<GroupOrder>,
    join: Option<(bool, u64)>,
    join_at_once: bool,
    read_after_ms: u64,
    read_every_ms: u64,
    cancel_after_objects: Option<u64>,
    cancel_after_ms: Option<u64>,
    cancel_request_only: bool,
    stay_ms: u64,
    answer: bool,
    answer_delay_ms: u64,
    stream_delay_ms: u64,
    write_every_ms: u64,
}

impl FetchFlags {
    /// Takes a flag with its value. `false` for a flag that is not about
    /// FETCH.
    fn read(&mut self, flag: &str, value: &str) -> Result<bool> {
        match flag {
            "--fetch-start" => self.start = Some(parse_location(value)?),
            "--fetch-end" => self.end = Some(parse_location(value)?),
            "--fetch-count" => self.count = Some(value.parse().context("--fetch-count")?),
            "--fill-timeout-ms" => {
                self.fill_timeout_ms = Some(value.parse().context("--fill-timeout-ms")?);
            }
            "--group-order" => {
                self.group_order = Some(match value {
                    "ascending" => GroupOrder::Ascending,
                    "descending" => GroupOrder::Descending,
                    other => bail!("unknown --group-order {other}"),
                });
            }
            "--join" => self.join = Some(parse_join(value)?),
            "--fetch-read-after-ms" => {
                self.read_after_ms = value.parse().context("--fetch-read-after-ms")?;
            }
            "--fetch-read-every-ms" => {
                self.read_every_ms = value.parse().context("--fetch-read-every-ms")?;
            }
            "--fetch-cancel-after-objects" => {
                self.cancel_after_objects =
                    Some(value.parse().context("--fetch-cancel-after-objects")?);
            }
            "--fetch-cancel-after-ms" => {
                self.cancel_after_ms = Some(value.parse().context("--fetch-cancel-after-ms")?);
            }
            "--fetch-stay-ms" => self.stay_ms = value.parse().context("--fetch-stay-ms")?,
            "--fetch-answer-delay-ms" => {
                self.answer_delay_ms = value.parse().context("--fetch-answer-delay-ms")?;
            }
            "--fetch-stream-delay-ms" => {
                self.stream_delay_ms = value.parse().context("--fetch-stream-delay-ms")?;
            }
            "--fetch-write-every-ms" => {
                self.write_every_ms = value.parse().context("--fetch-write-every-ms")?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Why the flags do not go together with each other or with `mode`.
    ///
    /// WHY refused and not ignored: a driver that mistypes a flag would run
    /// another test than it means to, and see it pass.
    fn misuse(&self, mode: &str) -> Option<&'static str> {
        let fetches = mode == "fetch";
        let joins = mode == "subscribe" && self.join.is_some();
        let of_a_range = self.start.is_some() || self.end.is_some() || self.count.is_some();
        let of_any_fetch = self.fill_timeout_ms.is_some()
            || self.group_order.is_some()
            || self.read_after_ms != 0
            || self.read_every_ms != 0
            || self.cancel_after_objects.is_some()
            || self.cancel_after_ms.is_some();
        if of_a_range && !fetches {
            return Some("--fetch-start, --fetch-end and --fetch-count belong to --mode fetch");
        }
        if self.join.is_some() && mode != "subscribe" {
            return Some("--join belongs to --mode subscribe");
        }
        if self.join_at_once && self.join.is_none() {
            return Some("--join-at-once needs --join");
        }
        if of_any_fetch && !fetches && !joins {
            return Some(
                "the flags of a FETCH belong to --mode fetch and to --mode subscribe with --join",
            );
        }
        if self
            .count
            .is_some_and(|count| count == 0 || count > MAX_FETCH_COUNT)
        {
            return Some("--fetch-count is 1 to 32");
        }
        if self.cancel_after_objects == Some(0) {
            return Some("--fetch-cancel-after-objects 0 is never reached");
        }
        // WHY not with a number of Objects: the task that counts them ends
        // when the number is reached, and the fetch stream it read is
        // stopped with it before anything can say that it is to be kept.
        let by_time_alone = self.cancel_after_ms.is_some() && self.cancel_after_objects.is_none();
        if self.cancel_request_only && !(fetches && by_time_alone) {
            return Some(
                "--fetch-cancel-request-only belongs to --mode fetch with --fetch-cancel-after-ms, and not with --fetch-cancel-after-objects",
            );
        }
        if self.stay_ms != 0 && !fetches {
            return Some("--fetch-stay-ms belongs to --mode fetch");
        }
        if self.answer && mode != "publish" && mode != "publish-namespace" {
            return Some("--answer-fetch belongs to the modes that publish");
        }
        let paced =
            self.answer_delay_ms != 0 || self.stream_delay_ms != 0 || self.write_every_ms != 0;
        if paced && !self.answer {
            return Some(
                "--fetch-answer-delay-ms, --fetch-stream-delay-ms and --fetch-write-every-ms need --answer-fetch",
            );
        }
        // A range that ends before it starts is the session's to refuse,
        // where the FETCH is made (`Session::fetch`).
        None
    }

    fn args(&self) -> FetchArgs {
        FetchArgs {
            count: self.count.unwrap_or(1),
            fill_timeout_ms: self.fill_timeout_ms,
            group_order: self.group_order,
            join: self.join.map(|(relative, start)| Join {
                relative,
                start,
                at_once: self.join_at_once,
            }),
            reading: Reading {
                after: Duration::from_millis(self.read_after_ms),
                every: Duration::from_millis(self.read_every_ms),
                cancel_after_objects: self.cancel_after_objects,
                cancel_after: self.cancel_after_ms.map(Duration::from_millis),
                cancel_request_only: self.cancel_request_only,
            },
            stay: Duration::from_millis(self.stay_ms),
            answering: self.answer.then(|| Pace {
                answer_after: Duration::from_millis(self.answer_delay_ms),
                stream_after: Duration::from_millis(self.stream_delay_ms),
                write_every: Duration::from_millis(self.write_every_ms),
            }),
        }
    }
}

/// Reads `<group>,<object>` of `--fetch-start` and `--fetch-end`.
fn parse_location(text: &str) -> Result<Location> {
    let (group, object) = text
        .split_once(',')
        .with_context(|| format!("{text} is not <group>,<object>"))?;
    Ok(Location {
        group: group.parse().context("the Group of a Location")?,
        object: object.parse().context("the Object of a Location")?,
    })
}

/// Reads `--subscription-filter`: `none` (an unfiltered subscription),
/// `largest-object`, `next-group-start`, `absolute-start:<group>,<object>`,
/// or `absolute-range:<group>,<object>,<end group>`.
///
/// WHY the End Group and not the End Group Delta the filter carries
/// (§5.1.2): whoever writes a run thinks of the last Group it wants.
fn parse_subscription_filter(text: &str) -> Result<Option<SubscriptionFilter>> {
    Ok(match text.split_once(':') {
        Some(("absolute-range", range)) => {
            let (start, end_group) = range
                .rsplit_once(',')
                .with_context(|| format!("{range} is not <group>,<object>,<end group>"))?;
            let start = parse_location(start)?;
            let end_group: u64 = end_group.parse().context("the End Group of a range")?;
            let end_group_delta = end_group
                .checked_sub(start.group)
                .with_context(|| format!("the End Group {end_group} is before the range starts"))?;
            Some(SubscriptionFilter::AbsoluteRange {
                start,
                end_group_delta,
            })
        }
        None if text == "none" => None,
        None if text == "largest-object" => Some(SubscriptionFilter::LargestObject),
        None if text == "next-group-start" => Some(SubscriptionFilter::NextGroupStart),
        Some(("absolute-start", start)) => Some(SubscriptionFilter::AbsoluteStart {
            start: parse_location(start)?,
        }),
        _ => bail!(
            "--subscription-filter {text} is none of none, largest-object, next-group-start, absolute-start:<group>,<object>, absolute-range:<group>,<object>,<end group>"
        ),
    })
}

/// Reads `relative:<n>` or `absolute:<n>` of `--join`.
fn parse_join(text: &str) -> Result<(bool, u64)> {
    let (kind, start) = text
        .split_once(':')
        .with_context(|| format!("--join {text} is not relative:<n> or absolute:<n>"))?;
    let relative = match kind {
        "relative" => true,
        "absolute" => false,
        other => bail!("--join {other} is neither relative nor absolute"),
    };
    Ok((
        relative,
        start.parse().context("the Joining Start of --join")?,
    ))
}

impl Args {
    /// Splits `a/b` into the namespace fields `a` and `b`. The slash is a
    /// convention of this command line, shared with moqtail's tools; the draft
    /// gives a Track Namespace no text form with slashes.
    fn track_namespace(&self) -> Result<TrackNamespace> {
        track_namespace_of(&self.namespace)
    }

    fn full_track_name(&self) -> Result<FullTrackName> {
        Ok(FullTrackName::new(
            self.track_namespace()?,
            self.track_name.as_bytes().to_vec(),
        )?)
    }
}

/// The Track Namespace a text names: its fields, separated by `/`.
fn track_namespace_of(text: &str) -> Result<TrackNamespace> {
    let fields = text
        .split('/')
        .filter(|field| !field.is_empty())
        .map(|field| field.as_bytes().to_vec())
        .collect();
    Ok(TrackNamespace::new(fields)?)
}

/// `--first-group`: a number, or `now` for the milliseconds since 1970.
///
/// WHY `now`: a publisher that comes back to a track it published before
/// has to begin above every Group it used (§2.4: "set the initial Group ID
/// to a unique value guaranteed to be larger than all previously used
/// groups. This can be done by choosing a Group ID based on the current
/// time"). A relay that still has the track gives its subscribers nothing
/// of a Group that is not larger than what it had. The pages of this
/// repository count from the time as well.
fn first_group_of(value: &str) -> Result<u64> {
    if value != "now" {
        return value.parse().context("--first-group");
    }
    let since_1970 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("the clock is before 1970")?;
    u64::try_from(since_1970.as_millis()).context("--first-group now")
}

/// The line that has to come after `publish-done`: whether it is
/// `publish-anew-same-alias`.
async fn next_publish_anew(stdin: &mut DriverLines) -> Result<bool> {
    match stdin.next_line().await?.as_deref().map(PublisherLine::read) {
        Some(PublisherLine::PublishAnew { same_alias }) => Ok(same_alias),
        other => {
            bail!("after publish-done comes publish-anew or publish-anew-same-alias, not {other:?}")
        }
    }
}

/// The line that has to come after `subgroup-again-longer` or
/// `subgroup-again-shorter`, before the second stream is sent.
async fn next_subgroup_again_second(stdin: &mut DriverLines) -> Result<()> {
    match stdin.next_line().await?.as_deref().map(PublisherLine::read) {
        Some(PublisherLine::SubgroupAgainSecond) => Ok(()),
        other => bail!("after subgroup-again-* comes subgroup-again-second, not {other:?}"),
    }
}

/// A line of the driver on a publisher's stdin.
#[derive(Debug, PartialEq, Eq)]
enum PublisherLine {
    /// Any other line: one Object is sent.
    Send,
    /// `republish`, `republish-same-alias`: `PUBLISH_DONE` and a new PUBLISH
    /// at once.
    Republish { same_alias: bool },
    /// `publish-done`: the `PUBLISH_DONE` alone.
    PublishDone,
    /// `publish-anew`, `publish-anew-same-alias`: the new PUBLISH after it.
    PublishAnew { same_alias: bool },
    /// `end-of-track`: Object 0 of the next Group with the status
    /// `END_OF_TRACK` and no payload (§2.1), as the other Objects are sent;
    /// what is sent after it is past the end of the track (§2.4.2).
    EndOfTrack,
    /// `subgroup-again-longer`, `subgroup-again-shorter`: the next Group on
    /// a stream that ends with a FIN, then, at the line
    /// `subgroup-again-second`, the same Subgroup on a second stream that
    /// ends with a FIN, with one Object more or one fewer: Objects 0, then 0
    /// and 1; or 0 and 1, then 0. The second and the third example of a
    /// malformed track (§2.4.2), for the interop run.
    SubgroupAgain { longer: bool },
    /// `subgroup-again-second`: the second stream of `SubgroupAgain`.
    SubgroupAgainSecond,
}

impl PublisherLine {
    fn read(line: &str) -> Self {
        match line {
            "republish" => Self::Republish { same_alias: false },
            "republish-same-alias" => Self::Republish { same_alias: true },
            "publish-done" => Self::PublishDone,
            "publish-anew" => Self::PublishAnew { same_alias: false },
            "publish-anew-same-alias" => Self::PublishAnew { same_alias: true },
            "end-of-track" => Self::EndOfTrack,
            "subgroup-again-longer" => Self::SubgroupAgain { longer: true },
            "subgroup-again-shorter" => Self::SubgroupAgain { longer: false },
            "subgroup-again-second" => Self::SubgroupAgainSecond,
            _ => Self::Send,
        }
    }

    /// The Object a line sends, as `Client::send_attending` takes it: the
    /// payload of the client for `Send`, none for `EndOfTrack` (§2.1).
    fn object(
        self,
        track_alias: u64,
        group_id: u64,
        payload: &[u8],
    ) -> (u64, u64, &[u8], ObjectStatus, u64) {
        match self {
            Self::EndOfTrack => (track_alias, group_id, &[], ObjectStatus::EndOfTrack, 1),
            _ => (track_alias, group_id, payload, ObjectStatus::Normal, 1),
        }
    }
}

/// Reads `<type>=<value>` of `--object-property`: both in the form the
/// `OBJECT` line prints them, the type in decimal, the value of an even type
/// in decimal and that of an odd type in hexadecimal (§1.4.3: an even type
/// has one integer, an odd type bytes).
fn parse_object_property(text: &str) -> Result<KeyValuePair> {
    let (ty, value) = text
        .split_once('=')
        .with_context(|| format!("--object-property {text} is not <type>=<value>"))?;
    let ty: u64 = ty.parse().context("the type of --object-property")?;
    let pair = if ty.is_multiple_of(2) {
        KeyValuePair::varint(ty, value.parse().context("the value of an even type")?)
    } else {
        KeyValuePair::bytes(ty, hex::decode(value).context("the value of an odd type")?)
    };
    pair.map_err(|error| anyhow!("--object-property {text}: {error:?}"))
}

/// Reads the value of `--publisher-priority`: a Publisher Priority, which is
/// one byte (§11.4.2), or `default` for a header without one.
fn parse_publisher_priority(text: &str) -> Result<Option<u8>> {
    if text == "default" {
        return Ok(None);
    }
    let priority = text
        .parse()
        .with_context(|| format!("--publisher-priority {text} is not 0 to 255 or `default`"))?;
    Ok(Some(priority))
}

/// The mode `--mode` names, with what that mode needs of the other flags.
fn read_mode(
    mode: Option<String>,
    payload: Option<&[u8]>,
    objects: u64,
    fetch: &FetchFlags,
) -> Result<Mode> {
    let payload = || -> Result<Vec<u8>> {
        payload
            .map(<[u8]>::to_vec)
            .context("--payload-hex or --payload-zeros is required to publish")
    };
    let mode = mode.context("--mode is required")?;
    if let Some(misuse) = fetch.misuse(&mode) {
        bail!("{misuse}");
    }
    Ok(match mode.as_str() {
        "subscribe" => Mode::Subscribe { objects },
        "fetch" => Mode::Fetch {
            start: fetch.start.context("--fetch-start is required to fetch")?,
            end: fetch.end.context("--fetch-end is required to fetch")?,
        },
        "publish" => Mode::Publish {
            payload: payload()?,
        },
        "publish-namespace" => Mode::PublishNamespace {
            payload: payload()?,
        },
        "subscribe-namespace" => Mode::SubscribeNamespace,
        "track-status" => Mode::TrackStatus,
        other => bail!("unknown --mode {other}"),
    })
}

/// Reads `--cert-sha256`: the SHA-256 of the relay's certificate, in hex.
fn read_cert_sha256(text: Option<String>) -> Result<[u8; 32]> {
    hex::decode(&text.context("--cert-sha256 is required")?)?
        .try_into()
        .map_err(|_| anyhow!("--cert-sha256 must be 32 bytes"))
}

/// The flags for how long this client waits for the peer, in milliseconds.
struct Waits {
    /// `--answer-wait-ms`: how long the answer to a request is waited for
    /// before the parameters of the request are looked at
    /// (`xmoq_moqt::params::answer_wait_ms`). Then the request alone is
    /// given up (`ANSWER_TIMED_OUT`), and the mode fails.
    answer: u64,
    /// `--setup-wait-ms`: how long the peer's SETUP is waited for. Then the
    /// session is closed with `CONTROL_MESSAGE_TIMEOUT`.
    setup: u64,
    /// `--open-wait-ms`: how long a request may wait for its stream to be
    /// opened, for the peer to allow one more. Then the request is not
    /// sent (`NOT_SENT`), and the mode fails.
    open: u64,
    /// `--rendezvous-timeout-ms`: the `RENDEZVOUS_TIMEOUT` of the SUBSCRIBEs
    /// this client sends (§10.2.6): how long a relay may hold one for a
    /// publisher to appear. 0, and without the flag: the parameter is left
    /// out.
    rendezvous: u64,
    /// `--idle-subgroup-stream-ms`: how long a subgroup stream of the peer
    /// may send nothing after its header while it is read
    /// ([`xmoq_moqt_io::IdleSubgroupStreamTimeout`]). Then it alone is
    /// stopped with `INTERNAL_ERROR`.
    idle_subgroup_stream: IdleSubgroupStreamTimeout,
}

impl Default for Waits {
    fn default() -> Self {
        Self {
            answer: xmoq_moqt::params::ANSWER_WAIT_MS,
            setup: client::SETUP_WAIT_MS,
            open: STREAM_OPEN_WAIT_MS,
            rendezvous: 0,
            idle_subgroup_stream: IdleSubgroupStreamTimeout::default(),
        }
    }
}

impl Waits {
    /// Takes the flag where it is one of these.
    fn read(&mut self, flag: &str, value: &str) -> Result<bool> {
        if flag == "--idle-subgroup-stream-ms" {
            self.idle_subgroup_stream =
                IdleSubgroupStreamTimeout::from_millis(millis(flag, value)?).with_context(
                    || format!("{flag} is from 1 to {}", IdleSubgroupStreamTimeout::MAX_MS),
                )?;
            return Ok(true);
        }
        let wait = match flag {
            "--answer-wait-ms" => &mut self.answer,
            "--setup-wait-ms" => &mut self.setup,
            "--open-wait-ms" => &mut self.open,
            "--rendezvous-timeout-ms" => &mut self.rendezvous,
            _ => return Ok(false),
        };
        *wait = millis(flag, value)?;
        Ok(true)
    }
}

/// What this client says and asks of new Groups on request (§10.2.13,
/// §12.6).
#[derive(Default)]
struct NewGroups {
    /// `--new-group-request <n>`: the `NEW_GROUP_REQUEST` of the SUBSCRIBEs
    /// this client sends. Without the flag the parameter is left out.
    ask: Option<u64>,
    /// `--dynamic-groups yes`: the PUBLISH of this client has the Track
    /// Property `DYNAMIC_GROUPS` with the value 1. WHY the client then only
    /// prints the requests it gets (`NEW_GROUP_REQUESTED`) and begins no
    /// Group for them: it publishes what its driver tells it to, and a
    /// driver reads the line.
    dynamic: bool,
}

impl NewGroups {
    /// Takes the flag where it is one of these.
    fn read(&mut self, flag: &str, value: &str) -> Result<bool> {
        match flag {
            "--new-group-request" => self.ask = Some(value.parse().context("--new-group-request")?),
            "--dynamic-groups" => self.dynamic = value == "yes",
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The Track Properties of a PUBLISH of this client.
    fn track_properties(&self) -> Result<Vec<KeyValuePair>> {
        if !self.dynamic {
            return Ok(Vec::new());
        }
        // DYNAMIC_GROUPS (0x30) = 1.
        Ok(vec![KeyValuePair::varint(0x30, 1)?])
    }
}

/// Answers an update of a subscription this client publishes that its
/// session applied, and says what it asked for.
async fn answer_update(publish_stream: &mut io::RequestStream, update: RequestEvent) -> Result<()> {
    let RequestEvent::SubscriptionUpdated {
        response,
        new_group_request,
        ..
    } = update
    else {
        bail!("not an update that was applied: {update:?}");
    };
    publish_stream.send(&response).await?;
    if let Some(group) = new_group_request {
        report::new_group_requested(group);
    }
    Ok(())
}

/// The value of a flag that takes milliseconds.
fn millis(flag: &str, value: &str) -> Result<u64> {
    value.parse().with_context(|| flag.to_string())
}

/// The payload of what is published: `--payload-hex` (its bytes) or
/// `--payload-zeros` (as many zero bytes).
///
/// WHY a payload by its size: a driver that wants a large one cannot pass
/// its hex, as one argument of a process holds at most 128 KiB on Linux
/// (`MAX_ARG_STRLEN`, E2BIG).
#[derive(Default)]
struct PayloadFlags {
    hex: Option<String>,
    zeros: Option<String>,
}

impl PayloadFlags {
    /// Takes the flag where it is one of these.
    fn read(&mut self, flag: &str, value: &str) -> bool {
        let to = match flag {
            "--payload-hex" => &mut self.hex,
            "--payload-zeros" => &mut self.zeros,
            _ => return false,
        };
        *to = Some(value.to_string());
        true
    }

    /// The payload the flags name; `None` for neither.
    fn payload(&self) -> Result<Option<Vec<u8>>> {
        Ok(match (&self.hex, &self.zeros) {
            (Some(_), Some(_)) => bail!("--payload-hex and --payload-zeros both name the payload"),
            (Some(text), None) => Some(hex::decode(text)?),
            (None, Some(size)) => Some(vec![0; size.parse().context("--payload-zeros")?]),
            (None, None) => None,
        })
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args> {
    let (mut mode, mut payload) = (None, PayloadFlags::default());
    let (mut objects, mut video) = (1, video::VideoFlags::default());
    let (mut url, mut cert_sha256) = (None, None);
    let (mut namespace, mut track_name) = (None, None);
    let (mut timeout_secs, mut publish_delay_ms, mut update_ms) = (20, 0, None);
    let (mut waits, mut new_groups) = (Waits::default(), NewGroups::default());
    let (mut subgroup_id, mut first_group) = (SubgroupId::Zero, 0);
    let mut publisher_priority = Some(0);
    let mut object_properties = Vec::new();
    let (mut serve_delay_ms, mut answer_delay_ms) = (0, 0);
    let mut subscription_filter = Some(SubscriptionFilter::LargestObject);
    let mut on_line = OnLine::default();
    let (mut fetch, mut auth) = (FetchFlags::default(), auth::AuthFlag::default());
    let (mut trace, mut catalog_video) = (false, false);

    while let Some(flag) = args.next() {
        let switch = match flag.as_str() {
            "--trace" => Some(&mut trace),
            "--publish-on-line" => Some(&mut on_line.publish),
            "--serve-on-line" => Some(&mut on_line.serve),
            "--subscribe-on-line" => Some(&mut on_line.subscribe),
            "--join-at-once" => Some(&mut fetch.join_at_once),
            "--catalog-video" => Some(&mut catalog_video),
            "--fetch-cancel-request-only" => Some(&mut fetch.cancel_request_only),
            "--answer-fetch" => Some(&mut fetch.answer),
            _ => None,
        };
        if let Some(switch) = switch {
            *switch = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| anyhow!("missing value for {flag}"))?;
        match flag.as_str() {
            "--mode" => mode = Some(value),
            "--objects" => objects = value.parse().context("--objects")?,
            "--url" => url = Some(value),
            "--cert-sha256" => cert_sha256 = Some(value),
            "--namespace" => namespace = Some(value),
            "--track" => track_name = Some(value),
            "--timeout-secs" => timeout_secs = value.parse().context("--timeout-secs")?,
            "--update-subgroup-timeout-ms" => update_ms = Some(millis(&flag, &value)?),
            "--subgroup-id" => {
                subgroup_id = match value.as_str() {
                    "zero" => SubgroupId::Zero,
                    "first-object" => SubgroupId::FirstObjectId,
                    "explicit" => SubgroupId::Explicit(0),
                    other => bail!("unknown --subgroup-id {other}"),
                };
            }
            "--first-group" => first_group = first_group_of(&value)?,
            "--publisher-priority" => publisher_priority = parse_publisher_priority(&value)?,
            "--object-property" => object_properties.push(parse_object_property(&value)?),
            "--serve-delay-ms" => serve_delay_ms = millis(&flag, &value)?,
            "--publish-delay-ms" => publish_delay_ms = millis(&flag, &value)?,
            "--fetch-answer-delay-ms" => answer_delay_ms = millis(&flag, &value)?,
            "--subscription-filter" => subscription_filter = parse_subscription_filter(&value)?,
            other => {
                if !(payload.read(other, &value)
                    || fetch.read(other, &value)?
                    || video.read(other, &value)?
                    || auth.read(other, &value)?
                    || waits.read(other, &value)?
                    || new_groups.read(other, &value)?)
                {
                    bail!("unknown flag {other}");
                }
            }
        }
    }

    let mode = read_mode(mode, payload.payload()?.as_deref(), objects, &fetch)?;

    Ok(Args {
        mode,
        url: url.context("--url is required")?,
        cert_sha256: read_cert_sha256(cert_sha256)?,
        namespace: namespace.context("--namespace is required")?,
        track_name: track_name.context("--track is required")?,
        video,
        auth,
        catalog_video,
        timeout_secs,
        waits,
        new_groups,
        update_subgroup_timeout_ms: update_ms,
        publish_delay: Duration::from_millis(publish_delay_ms),
        subgroup_id,
        first_group,
        publisher_priority,
        object_properties,
        serve_delay: Duration::from_millis(serve_delay_ms),
        answer_delay: Duration::from_millis(answer_delay_ms),
        subscription_filter,
        on_line,
        fetch: fetch.args(),
        trace: if trace {
            Trace::new(report::trace)
        } else {
            Trace::default()
        },
    })
}

/// The Message Parameters of a request this client sends on a stream of
/// its own: what the time its answer gets goes by.
fn parameters_of(message: &ControlMessage) -> &[Parameter] {
    match message {
        ControlMessage::Publish(publish) => &publish.parameters,
        ControlMessage::PublishNamespace(namespace) => &namespace.parameters,
        ControlMessage::Subscribe(subscribe) => &subscribe.parameters,
        ControlMessage::Fetch(fetch) => &fetch.parameters,
        _ => &[],
    }
}

/// The Error Reason of the `PUBLISH_DONE` that ends a PUBLISH nobody
/// answered; `@xmoq/moqt` sends the same.
const NO_ANSWER_IN_TIME: &str = "no answer in time";

/// Says that the answer to a request did not come in time
/// (`ANSWER_TIMED_OUT`), and gives the error its caller fails with.
fn not_answered(label: &str, waited_ms: u64) -> anyhow::Error {
    report::answer_timed_out(label, waited_ms);
    anyhow!("the peer did not answer the {label} request within {waited_ms} ms")
}

/// What a response other than the expected one means for the caller.
fn refused(what: &str, event: RequestEvent) -> anyhow::Error {
    match event {
        RequestEvent::Rejected(RequestError { error_code, .. })
            if error_code == request_error_code::GOING_AWAY =>
        {
            GoingAway(what.to_string()).into()
        }
        RequestEvent::Rejected(RequestError {
            error_code,
            retry_interval,
            error_reason,
            ..
        }) => anyhow!(
            "{what} rejected: error_code={error_code:#x} retry_interval={retry_interval} reason={:?}",
            String::from_utf8_lossy(error_reason.as_bytes()),
        ),
        other => anyhow!("unexpected response to {what}: {other:?}"),
    }
}

/// The next event of a request stream, or never when the stream is gone.
async fn next_request_event(stream: &mut Option<RequestStream>) -> Result<RequestStreamEvent> {
    match stream {
        Some(stream) => stream.next_event().await,
        None => std::future::pending().await,
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// The Largest Object of the track a client publishes (§10.2.11): the
/// Location of the last Object it began to send (§5.1.2 counts an Object
/// from its first byte). Each Object is Object 0 of a Group of its own, and
/// the Groups go up.
#[derive(Debug, Default)]
struct Largest(Option<Location>);

impl Largest {
    /// Notes that the Object of the Group `group_id` is being sent.
    fn sent(&mut self, group_id: u64) {
        self.0 = Some(Location {
            group: group_id,
            object: 0,
        });
    }

    /// `LARGEST_OBJECT`, for a message that says where the track is. None
    /// before the first Object: "If omitted from a message, the sending
    /// endpoint has not published or received any Objects in the Track."
    fn parameter(&self) -> Option<Parameter> {
        self.0.map(Parameter::LargestObject)
    }
}

/// One session and what the two modes need to use it.
struct Client<'a> {
    connection: &'a Arc<WtConnection>,
    session: Session,
    objects: DataInbox,
    /// Stops the readers of subgroup streams this client no longer wants.
    stops: StreamStops,
    /// The Track Alias of every subgroup stream that is being read.
    open_streams: HashMap<u64, u64>,
    /// The requests the peer opens, in the mode that serves them.
    peer_requests: Option<io::PeerRequests>,
    /// Requests of the peer this client has said its last on while their
    /// stream was still open, by that stream: subscriptions it ended with
    /// `PUBLISH_DONE`, and requests that were refused, by this client or by
    /// its session. WHY kept: what the peer still says there has to reach
    /// the session (a `REQUEST_UPDATE` uses up a Request ID, §10.1), and the
    /// session forgets the request only when it is told that the stream is
    /// gone.
    said_the_last: HashMap<u64, PeerRequestId>,
    /// Taken when the answer to `--update-subgroup-timeout-ms` comes; the
    /// wait for it ends then ([`Client::send_update`]).
    update_answer: Option<tokio::sync::oneshot::Sender<()>>,
    /// The Largest Object of the track this client publishes.
    largest: Largest,
    /// The refusals that wait for `--fetch-answer-delay-ms`. They go with the
    /// client: a session that is over sends nothing more.
    later: tokio::task::JoinSet<()>,
    /// The Objects this client sent, for a FETCH.
    published: Published,
    /// The FETCH requests of the peer that are being answered.
    answers: Answers,
    args: &'a Args,
}

/// The FETCH among the requests of the peer that a client with
/// `--answer-fetch` answers from what it published. Without the flag none:
/// every FETCH is refused with `NOT_SUPPORTED` ([`refusal_code`]), which
/// drivers count on.
fn fetch_to_answer(answering: Option<Pace>, request: &PeerRequest) -> Option<&Fetch> {
    match request {
        PeerRequest::Fetch(fetch) if answering.is_some() => Some(fetch),
        _ => None,
    }
}

/// The code a client that publishes refuses a request of the peer with,
/// where it serves none of that kind. `serves`: the mode that serves a
/// SUBSCRIBE for its track, for which this is asked about any other.
///
/// WHY `NOT_SUPPORTED` for a PUBLISH and a `PUBLISH_NAMESPACE` in every mode:
/// the client takes neither of any track (§4: "Limited endpoints SHOULD
/// respond to any unsupported messages with the appropriate `NOT_SUPPORTED`
/// error code"). WHY NOT `UNINTERESTED`: that is a subscriber's that takes
/// PUBLISH and does not want this track (§10.10), which would read as if
/// another track were taken.
///
/// A SUBSCRIBE read before this client has sent its PUBLISH gets
/// `NOT_SUPPORTED` too. No rule is broken: the subscription §5.1 means ("an
/// existing subscription in Pending (publisher) state") begins with the
/// PUBLISH, and once that is sent the session answers
/// `DUPLICATE_SUBSCRIPTION`, as `@xmoq/moqt` does. The relay waits for a
/// PUBLISH after `DUPLICATE_SUBSCRIPTION` and not after `NOT_SUPPORTED`, so a
/// subscriber whose SUBSCRIBE reaches this client through the relay before
/// its PUBLISH is on the way gets `DOES_NOT_EXIST`; the track is there a
/// moment later.
fn refusal_code(serves: bool, request: &PeerRequest) -> u64 {
    match request {
        // A track other than the one it announced.
        PeerRequest::Subscribe(_) if serves => request_error_code::DOES_NOT_EXIST,
        // TRACK_STATUS: the client answers none (decided on 2026-10-24),
        // and a relay passes the refusal on.
        PeerRequest::Subscribe(_)
        | PeerRequest::Fetch(_)
        | PeerRequest::SubscribeNamespace(_)
        | PeerRequest::TrackStatus(_)
        | PeerRequest::Publish(_)
        | PeerRequest::PublishNamespace(_) => request_error_code::NOT_SUPPORTED,
    }
}

/// What a client that publishes turns to between the things its mode does.
enum Attention {
    Peer(PeerRequestStreamEvent<<WtConnection as Connection>::Send>),
    /// A task that answers a FETCH has something to tell.
    Note(answers::Note),
    /// A unidirectional stream of the peer.
    Data(UniEvent),
}

/// The next thing a client that publishes has to turn to.
async fn attention(
    peer_requests: &mut io::PeerRequests,
    answers: &mut Answers,
    objects: &mut DataInbox,
) -> Result<Attention> {
    tokio::select! {
        event = peer_requests.next_event() => Ok(Attention::Peer(event?)),
        note = answers.next_note() => Ok(Attention::Note(note)),
        event = objects.next_event() => Ok(Attention::Data(event?)),
    }
}

/// A SUBSCRIBE of the peer this client accepted.
struct Serving {
    request: PeerRequestId,
    /// The peer's request stream, as the readers name it.
    stream: u64,
    send: <WtConnection as Connection>::Send,
    track_alias: u64,
    /// Subgroup streams opened for the subscription, for `PUBLISH_DONE`.
    streams: u64,
    /// A subgroup stream of the subscription is being written.
    sending: bool,
    /// The status of a `PUBLISH_DONE` that waits for the stream being
    /// written to be closed (§10.11: "A sender MUST NOT send `PUBLISH_DONE`
    /// until it has closed all streams it will ever open").
    done_due: Option<u64>,
}

/// The `PUBLISH_NAMESPACE` of this client, once it is answered with
/// `REQUEST_OK`, and its stream while the namespace is published.
struct Announced {
    request: RequestId,
    stream: Option<RequestStream>,
}

impl Announced {
    /// What comes on the stream, or a `STOP_SENDING` for this client's
    /// direction; never, once the namespace is withdrawn.
    async fn next_event(&mut self) -> Result<RequestStreamEvent> {
        match &mut self.stream {
            Some(stream) => stream.next_event_or_stopped().await,
            None => std::future::pending().await,
        }
    }
}

/// Completes with the code of the `STOP_SENDING` the subscriber sent on the
/// stream of the subscription that is being served; never, when none is.
async fn stopped(serving: &mut Option<Serving>) -> Option<u32> {
    match serving {
        Some(serving) => match serving.send.stopped().await {
            xmoq_moqt_io::WriteError::Stopped(code) => code,
            // A lost connection is reported where the session is watched.
            xmoq_moqt_io::WriteError::Lost(_) => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
}

impl Client<'_> {
    /// `--mode subscribe`: to the track of `--track`, or with
    /// `--catalog-video` to the video track its catalog names.
    /// The mode `subscribe-namespace`: `SUBSCRIBE_NAMESPACE` for the
    /// namespace of `--namespace` as the prefix, and what comes on its stream,
    /// until stdin ends; then the request is ended with a FIN (§6.1: "can be
    /// cancelled by closing the stream with either a FIN or `RESET_STREAM`").
    /// The mode `track-status`: one `TRACK_STATUS`, whose answer ends it.
    /// The peer closes the stream with a FIN after its answer (§10.14), and
    /// this client finishes its own direction.
    async fn track_status(&mut self) -> Result<()> {
        let parameters = self.args.auth.with(vec![]);
        let (request_id, message) = self
            .session
            .track_status(self.args.full_track_name()?, parameters.clone())?;
        let mut stream = self
            .open_request(request_id, &message, "track-status")
            .await?;
        let answer = self
            .response_stopping_data(request_id, &mut stream, "track-status", &parameters)
            .await?;
        stream.finish();
        match answer {
            RequestEvent::TrackStatusAnswered {
                parameters,
                track_properties,
            } => report::track_status_ok(&parameters, &track_properties),
            RequestEvent::Rejected(error) => report::track_status_refused(error.error_code),
            other => return Err(refused("TRACK_STATUS", other)),
        }
        Ok(())
    }

    async fn watch_namespaces(&mut self) -> Result<()> {
        let parameters = self.args.auth.with(vec![]);
        let (request_id, message) = self
            .session
            .subscribe_namespace(self.args.track_namespace()?, parameters.clone())?;
        let mut stream = self
            .open_request(request_id, &message, "subscribe-namespace")
            .await?;
        match self
            .response_stopping_data(request_id, &mut stream, "subscribe-namespace", &parameters)
            .await?
        {
            RequestEvent::NamespacesSubscribed => report::namespaces_subscribed(),
            other => {
                stream.cancel();
                return Err(refused("SUBSCRIBE_NAMESPACE", other));
            }
        }
        let mut stdin = BufReader::new(tokio::io::stdin()).lines();
        loop {
            tokio::select! {
                line = stdin.next_line() => if line?.is_none() {
                    stream.finish();
                    return Ok(());
                },
                // Nothing on a data stream was asked for (`response_stopping_data`).
                event = self.objects.next_event() => self.on_unasked_stream(event?)?,
                event = stream.next_event() => match event? {
                    RequestStreamEvent::Message(message) => {
                        match on_request(&mut self.session, request_id, message)? {
                            RequestEvent::Namespace(namespace) => report::namespace(&namespace),
                            RequestEvent::NamespaceDone(namespace) => {
                                report::namespace_done(&namespace);
                            }
                            RequestEvent::TooManyNamespaces { limit } => {
                                stream.cancel();
                                bail!("named more than {limit} namespaces");
                            }
                            other => bail!("unexpected on the SUBSCRIBE_NAMESPACE stream: {other:?}"),
                        }
                    }
                    // §10.18: "When a subscriber receives a stream reset or FIN
                    // on a SUBSCRIBE_NAMESPACE response stream, it SHOULD treat
                    // this as though each active namespace received a
                    // NAMESPACE_DONE."
                    RequestStreamEvent::Closed(how) => {
                        bail!("the peer ended the SUBSCRIBE_NAMESPACE stream: {how}")
                    }
                },
            }
        }
    }

    async fn subscribe(&mut self, wanted: u64) -> Result<()> {
        let join = self.args.fetch.join;
        if !self.args.catalog_video {
            let track = self.args.full_track_name()?;
            return self.subscribe_to(track, join, Some(wanted)).await;
        }
        // MSF §5: "Subscribers accessing the catalog MUST use SUBSCRIBE with
        // a Joining FETCH (offset = 0) in order to obtain the latest
        // complete catalog".
        catalog::want();
        let latest = Join {
            relative: true,
            start: 0,
            at_once: false,
        };
        let catalog_track = self.args.full_track_name()?;
        self.subscribe_to(catalog_track, Some(latest), None).await?;
        let payload = catalog::take().context("the catalog track ended before a catalog came")?;
        let tracks = xmoq_msf::read_catalog(&payload)?;
        let video = xmoq_msf::first_video_track(&tracks, "avc")
            .context("the catalog has no H.264 video track with LOC packaging")?;
        report::catalog_track(video);
        // §5.2.2: a track without a namespace is in the catalog track's.
        let namespace = match &video.namespace {
            Some(namespace) => track_namespace_of(namespace)?,
            None => self.args.track_namespace()?,
        };
        let track = FullTrackName::new(namespace, video.name.as_bytes().to_vec())?;
        self.subscribe_to(track, join, Some(wanted)).await
    }

    /// Subscribes to `track` and takes its Objects: `wanted` of them, or
    /// with `None` until a catalog came (`--catalog-video`).
    async fn subscribe_to(
        &mut self,
        track: FullTrackName,
        join: Option<Join>,
        wanted: Option<u64>,
    ) -> Result<()> {
        let parameters = [
            Some(Parameter::Forward(true)),
            Some(Parameter::SubscriberPriority(128)),
            self.args
                .subscription_filter
                .map(Parameter::SubscriptionFilter),
            Some(Parameter::GroupOrder(GroupOrder::Ascending)),
            Some(self.args.waits.rendezvous)
                .filter(|wait| *wait > 0)
                .map(Parameter::RendezvousTimeout),
            self.args.new_groups.ask.map(Parameter::NewGroupRequest),
        ];
        let parameters = self
            .args
            .auth
            .with(parameters.into_iter().flatten().collect());
        let (request_id, message) = self.session.subscribe(track, parameters.clone())?;
        let mut fetches = Fetches::new(self.args.fetch.reading);
        let joining = join.map(|join| {
            let joining_request_id = request_id.value();
            let range = if join.relative {
                FetchRange::RelativeJoining {
                    joining_request_id,
                    joining_start: join.start,
                }
            } else {
                FetchRange::AbsoluteJoining {
                    joining_request_id,
                    joining_start: join.start,
                }
            };
            (range, join.at_once)
        });
        let mut stream = self.open_request(request_id, &message, "subscribe").await?;
        if let Some((range, true)) = &joining {
            self.send_fetch(&mut fetches, range.clone()).await?;
        }
        let event = self
            .response(request_id, &mut stream, "subscribe", &parameters)
            .await;
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                self.session.cancel(request_id);
                stream.cancel();
                return Err(error);
            }
        };
        let track_alias = match event {
            RequestEvent::SubscribeEstablished { track_alias, .. } => track_alias,
            RequestEvent::TrackNotProcessed {
                refusal,
                track_alias,
            } => {
                // §2.5.1 and §12.7: the session already ended the request;
                // the stream is cancelled to tell the publisher, and so are
                // the subgroup streams it may have opened already.
                self.stop_streams_of(track_alias);
                match refusal {
                    TrackRefusal::Unsupported(property_type) => {
                        stream.cancel();
                        bail!(
                            "the track needs property {property_type:#x}, which this client lacks"
                        );
                    }
                    // §2.4.2: "it MUST cancel any corresponding subscription".
                    TrackRefusal::Malformed => {
                        stream.cancel_with(stream_reset_code::MALFORMED_TRACK);
                        bail!(
                            "the immutable properties of the SUBSCRIBE_OK make the track malformed"
                        );
                    }
                }
            }
            other => {
                stream.cancel();
                return Err(refused("SUBSCRIBE", other));
            }
        };
        report::subscribed(track_alias);
        if let Some(milliseconds) = self.args.update_subgroup_timeout_ms {
            self.send_update(request_id, &mut stream, milliseconds)
                .await?;
        }
        if let Some((range, false)) = joining {
            self.send_fetch(&mut fetches, range).await?;
        }
        let received = self
            .receive_objects(request_id, stream, track_alias, wanted, &mut fetches)
            .await;
        // The subscription can be over before its Joining FETCH is.
        self.leave_fetches(&mut fetches);
        received
    }

    /// `--update-subgroup-timeout-ms`: sends the `REQUEST_UPDATE` for the
    /// subscription, and starts the wait for its answer.
    ///
    /// The answer gets the time a request gets (`--answer-wait-ms`), counted
    /// from when the update is written. When it is over
    /// `ANSWER_TIMED_OUT request=update` is printed and nothing else
    /// happens. WHY the subscription goes on: an update is no request of
    /// its own that could be cancelled alone (§10.9 has it on the stream
    /// of the SUBSCRIBE), and giving the subscription up for it would cost
    /// more than the update was for. The browser client does the same
    /// (`Subscription.update`).
    async fn send_update(
        &mut self,
        request_id: RequestId,
        stream: &mut RequestStream,
        milliseconds: u64,
    ) -> Result<()> {
        let parameters = vec![Parameter::SubgroupDeliveryTimeout(milliseconds)];
        let wait = xmoq_moqt::params::answer_wait_ms(self.args.waits.answer, &parameters);
        let update = self.session.update_subscription(request_id, parameters)?;
        stream.send(&update).await?;
        let (answered, answer) = tokio::sync::oneshot::channel();
        self.update_answer = Some(answered);
        tokio::spawn(async move {
            // An error: the client is gone, and nobody waits any more.
            if tokio::time::timeout(Duration::from_millis(wait), answer)
                .await
                .is_err()
            {
                report::answer_timed_out("update", wait);
            }
        });
        Ok(())
    }

    /// What else the stream of an Established SUBSCRIBE carries: the answers
    /// to `--update-subgroup-timeout-ms`, also one that comes after its
    /// time. After a refusal the publisher's `PUBLISH_DONE` follows
    /// (§10.9.1).
    fn update_answered(&mut self, event: RequestEvent) -> Result<()> {
        match event {
            RequestEvent::UpdateAccepted { .. } => report::update_answered(None),
            RequestEvent::UpdateFailed(error) => report::update_answered(Some(error.error_code)),
            other => bail!("unexpected message on the subscribe stream: {other:?}"),
        }
        if let Some(answered) = self.update_answer.take() {
            // An error: its time was over already.
            let _ = answered.send(());
        }
        Ok(())
    }

    /// `--mode fetch`: sends the Standalone FETCH requests and sees each to
    /// its end.
    async fn fetch(&mut self, start: Location, end: Location) -> Result<()> {
        let mut fetches = Fetches::new(self.args.fetch.reading);
        let full_track_name = self.args.full_track_name()?;
        for _ in 0..self.args.fetch.count {
            let range = FetchRange::Standalone {
                full_track_name: full_track_name.clone(),
                start,
                end,
            };
            self.send_fetch(&mut fetches, range).await?;
        }
        while !fetches.all_over() {
            tokio::select! {
                note = fetches.next_note() => self.on_fetch_note(&mut fetches, note)?,
                event = self.objects.next_event() => match event? {
                    UniEvent::FetchOpened { stream, header } => {
                        self.on_fetch_stream(&mut fetches, stream, header)?;
                    }
                    // This client subscribed to nothing (§11.4.2).
                    UniEvent::SubgroupOpened { stream, header } => {
                        report::ignored(header.track_alias);
                        self.stops.stop(stream, stream_reset_code::CANCELLED);
                    }
                    _ => {}
                },
            }
        }
        // `fetches` is held meanwhile, and with it the streams it kept.
        tokio::time::sleep(self.args.fetch.stay).await;
        if fetches.unanswered {
            bail!("the peer did not answer a FETCH in time");
        }
        Ok(())
    }

    /// Sends a FETCH on a new request stream and leaves the stream to
    /// `fetches`.
    async fn send_fetch(&mut self, fetches: &mut Fetches, range: FetchRange) -> Result<()> {
        let mut parameters = Vec::new();
        if let Some(millis) = self.args.fetch.fill_timeout_ms {
            parameters.push(Parameter::FillTimeout(millis));
        }
        if let Some(group_order) = self.args.fetch.group_order {
            parameters.push(Parameter::GroupOrder(group_order));
        }
        let parameters = self.args.auth.with(parameters);
        let answer_wait = Duration::from_millis(xmoq_moqt::params::answer_wait_ms(
            self.args.waits.answer,
            &parameters,
        ));
        let (request_id, message) = self.session.fetch(range, parameters)?;
        let label = format!("fetch#{}", request_id.value());
        let stream = self.open_request(request_id, &message, &label).await?;
        report::fetch_sent(request_id.value());
        // §10.2.8: without the parameter the Groups come in Ascending order.
        let group_order = self.args.fetch.group_order.unwrap_or(GroupOrder::Ascending);
        fetches.add(request_id, group_order, stream, answer_wait);
        Ok(())
    }

    /// A fetch stream of the peer opened: the session says what it belongs
    /// to.
    fn on_fetch_stream(
        &mut self,
        fetches: &mut Fetches,
        stream: u64,
        header: FetchHeader,
    ) -> Result<()> {
        let opened = self.session.on_fetch_stream(header.request_id)?;
        // Taken in every case: a body that is dropped stops its stream,
        // which is what a stream for a FETCH that is over gets.
        let body = self.stops.take_fetch_body(stream);
        if let (FetchStreamOpened::Fetch(request_id), Some(body)) = (opened, body)
            && let Some(index) = fetches.index_of(request_id)
        {
            fetches.read(index, header, body);
        }
        Ok(())
    }

    /// What a task of a FETCH reported: what it means is read here, and what
    /// follows from it is `fetching::step`'s to say.
    fn on_fetch_note(&mut self, fetches: &mut Fetches, note: Note) -> Result<()> {
        let (index, happened) = match note {
            Note::Message(index, message) => {
                let Some(fetch) = fetches.sent.get(index) else {
                    return Ok(());
                };
                let id = fetch.id.value();
                // A FETCH that is over hears the peer where it came to its
                // end or was refused: a message after the last is one the
                // session finds out. Not where this client gave it up.
                if !fetch.progress.hears_the_peer() {
                    return Ok(());
                }
                let happened = match on_request(&mut self.session, fetch.id, message)? {
                    RequestEvent::FetchEstablished {
                        end_of_track,
                        end_location,
                        track_properties,
                    } => {
                        report::fetch_ok(id, end_location, end_of_track, &track_properties);
                        Happened::Answered
                    }
                    RequestEvent::Rejected(error) => {
                        report::fetch_refused(id, error.error_code);
                        Happened::Refused
                    }
                    // §2.5.1 and §12.7: the session ended the request; the
                    // streams are cancelled to tell the publisher.
                    RequestEvent::FetchNotProcessed {
                        refusal: TrackRefusal::Unsupported(_),
                    } => Happened::UnsupportedTrackProperty,
                    RequestEvent::FetchNotProcessed {
                        refusal: TrackRefusal::Malformed,
                    } => Happened::MalformedTrack,
                    other => bail!("unexpected message on a fetch request stream: {other:?}"),
                };
                (index, happened)
            }
            Note::RequestClosed(index, how) => (index, Happened::RequestClosed(how)),
            Note::CancelDue(index) => (index, Happened::GaveUp),
            Note::AnswerDue(index) => (index, Happened::NotAnsweredInTime),
            Note::RequestBroken(error) => return Err(error),
            Note::Body(index, end) => {
                let happened = match end {
                    BodyEnd::Fin => Happened::StreamEnded(StreamEnded::Fin),
                    BodyEnd::Reset(code) => Happened::StreamEnded(StreamEnded::Reset(code)),
                    BodyEnd::Lost => bail!("the connection was lost while a fetch stream was read"),
                    BodyEnd::Stopped => Happened::StreamStopped,
                    BodyEnd::GaveUp => Happened::GaveUp,
                    // By what the error ends, and not by its name: one that
                    // ends the session closes it, any other ends this
                    // FETCH (§2.4.2 for a track that is at fault).
                    BodyEnd::Broken(error) => {
                        if matches!(error.scope(), ErrorScope::Session(_)) {
                            return Err(error.into());
                        }
                        if let Some(fetch) = fetches.sent.get(index) {
                            report::fetch_stream_refused(fetch.id.value(), &error);
                        }
                        Happened::StreamRefused
                    }
                };
                (index, happened)
            }
        };
        self.fetch_happened(fetches, index, happened);
        Ok(())
    }

    /// Applies what happened to a FETCH: to the session, to its streams, and
    /// to stdout, where every FETCH gets one `FETCH_ENDED`.
    fn fetch_happened(&mut self, fetches: &mut Fetches, index: usize, happened: Happened) {
        let Some(fetch) = fetches.sent.get_mut(index) else {
            return;
        };
        let step = fetching::step(fetch.progress, happened);
        fetch.progress = step.progress;
        let (id, answer_wait_ms) = (fetch.id.value(), fetch.answer_wait_ms);
        match step.tell {
            Some(Tell::Cancel) => self.session.cancel(fetch.id),
            Some(Tell::StreamClosed) => self.session.on_fetch_stream_closed(fetch.id),
            None => {}
        }
        let ended = step.ended;
        if let Some(ended) = ended {
            fetch.print_ended(ended);
            let code = match happened {
                Happened::MalformedTrack => stream_reset_code::MALFORMED_TRACK,
                _ => stream_reset_code::CANCELLED,
            };
            fetches.end(
                index,
                ended.cancel.then_some(code),
                happened == Happened::GaveUp,
            );
        }
        if ended.is_some() || happened == Happened::Answered {
            fetches.stop_answer_timer(index);
        }
        if ended.is_some() && happened == Happened::NotAnsweredInTime {
            report::answer_timed_out(&format!("fetch#{id}"), answer_wait_ms);
            fetches.unanswered = true;
        }
    }

    /// The client leaves: every FETCH that is still open is given up, and
    /// said to be.
    fn leave_fetches(&mut self, fetches: &mut Fetches) {
        for index in 0..fetches.sent.len() {
            self.fetch_happened(fetches, index, Happened::Left);
        }
    }

    /// Opens a request stream and sends the request on it. On a failure the
    /// request is cancelled in the session.
    async fn open_request(
        &mut self,
        request_id: RequestId,
        message: &ControlMessage,
        label: &str,
    ) -> Result<RequestStream> {
        // WHY a time for it, and the request alone that fails: a stream is
        // opened when the peer allows one more, and a peer that allows none
        // would hold the request for good. Nothing was sent, so nothing is
        // reset; the session is not closed over it. `@xmoq/moqt` gives a
        // request the same time (`openStreamTimeoutMs`), and the draft
        // names none (docs/moqt-implementation-notes.md).
        let wait = Duration::from_millis(self.args.waits.open);
        let opening = RequestStream::open(self.connection, &self.args.trace, label);
        let opened = tokio::time::timeout(wait, opening)
            .await
            .unwrap_or_else(|_| {
                report::not_sent(label, self.args.waits.open);
                Err(anyhow!(
                    "the peer allowed no stream for the {label} request within {} ms",
                    self.args.waits.open
                ))
            });
        let mut stream = match opened {
            Ok(stream) => stream,
            Err(error) => {
                self.session.cancel(request_id);
                return Err(error);
            }
        };
        if let Err(error) = stream.send(message).await {
            self.session.cancel(request_id);
            stream.cancel();
            return Err(error);
        }
        Ok(stream)
    }

    /// The event the response to a request produces.
    async fn response(
        &mut self,
        request_id: RequestId,
        stream: &mut RequestStream,
        label: &str,
        parameters: &[Parameter],
    ) -> Result<RequestEvent> {
        let event = self
            .in_time(label, parameters, stream.next_event())
            .await??;
        self.on_response(request_id, event, label)
    }

    /// [`Self::response`], for a mode that asks for nothing on data streams:
    /// what they bring meanwhile is stopped ([`Self::on_unasked_stream`]).
    /// WHY taken at all: the readers wait while what they brought is not
    /// taken, and the control stream comes through them too, a GOAWAY and
    /// its end among it (`io::start_readers`).
    async fn response_stopping_data(
        &mut self,
        request_id: RequestId,
        stream: &mut RequestStream,
        label: &str,
        parameters: &[Parameter],
    ) -> Result<RequestEvent> {
        let wait = xmoq_moqt::params::answer_wait_ms(self.args.waits.answer, parameters);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
        let event = loop {
            tokio::select! {
                answer = tokio::time::timeout_at(deadline, stream.next_event()) => {
                    break answer.map_err(|_| not_answered(label, wait))??;
                }
                event = self.objects.next_event() => self.on_unasked_stream(event?)?,
            }
        };
        self.on_response(request_id, event, label)
    }

    /// `answer`, where it comes within the time the answer to a request
    /// with these parameters gets (`answer_wait_ms`); else an error, after
    /// `ANSWER_TIMED_OUT`. The caller cancels the request, as for any error
    /// of it: its stream is reset with `CANCELLED`, and the session is not
    /// closed over it.
    ///
    /// WHY the request and not the session, which §3.5 has a code for
    /// (`CONTROL_MESSAGE_TIMEOUT`, "the peer took too long to respond to a
    /// control message"): a relay answers a SUBSCRIBE when the publisher
    /// behind it has, so one slow publisher would end every other request
    /// of the session. `@xmoq/moqt` does the same
    /// (`docs/moqt-implementation-notes.md`).
    async fn in_time<T>(
        &self,
        label: &str,
        parameters: &[Parameter],
        answer: impl Future<Output = T>,
    ) -> Result<T> {
        let wait = xmoq_moqt::params::answer_wait_ms(self.args.waits.answer, parameters);
        let Ok(answer) = tokio::time::timeout(Duration::from_millis(wait), answer).await else {
            return Err(not_answered(label, wait));
        };
        Ok(answer)
    }

    /// The event that what came on the stream of a request makes of it.
    fn on_response(
        &mut self,
        request_id: RequestId,
        event: RequestStreamEvent,
        label: &str,
    ) -> Result<RequestEvent> {
        match event {
            RequestStreamEvent::Message(response) => {
                Ok(on_request(&mut self.session, request_id, response)?)
            }
            RequestStreamEvent::Closed(how) => {
                bail!("the peer closed the {label} stream without a response: {how}")
            }
        }
    }

    /// §2.4.2: the subscription and the FETCH requests of a malformed track
    /// are cancelled, the FETCH requests here. Whether the subscription
    /// `request_id` was among them.
    fn cancel_malformed(
        &mut self,
        fetches: &mut Fetches,
        track_alias: u64,
        request_id: RequestId,
    ) -> bool {
        let mut subscription_over = false;
        for cancelled in self.session.cancel_malformed(track_alias) {
            match cancelled {
                MalformedRequest::Subscribe(request) => subscription_over |= request == request_id,
                MalformedRequest::Fetch(request) => {
                    if let Some(index) = fetches.index_of(request) {
                        self.fetch_happened(fetches, index, Happened::MalformedTrack);
                    }
                }
                // The client takes no PUBLISH of the peer.
                MalformedRequest::Publish(_) => {}
            }
        }
        subscription_over
    }

    /// The writer of `--annex-b-out`. Not for the catalog track (`wanted`
    /// is `None`): its Objects are no frames.
    fn video_writer(&self, wanted: Option<u64>) -> Result<Option<video::AnnexBWriter>> {
        if wanted.is_none() {
            return Ok(None);
        }
        if self.args.video.viewers > 0 {
            return Ok(Some(video::AnnexBWriter::counting()));
        }
        video::AnnexBWriter::for_flag(self.args.video.annex_b_out.as_deref())
    }

    /// Prints Objects of the subscription until `wanted` arrived or the
    /// subscription ended. The request stream is read all the while: it carries
    /// `PUBLISH_DONE`, and the peer cancels the subscription by closing it
    /// (§11.4.1).
    async fn receive_objects(
        &mut self,
        request_id: RequestId,
        stream: RequestStream,
        track_alias: u64,
        wanted: Option<u64>,
        fetches: &mut Fetches,
    ) -> Result<()> {
        let mut stream = Some(stream);
        let mut drain_deadline = None;
        let mut drain_timed_out = false;
        let mut received = 0;
        let mut video = self.video_writer(wanted)?;
        let more = |received| wanted.map_or_else(|| !catalog::came(), |wanted| received < wanted);
        let wanted = wanted.unwrap_or(0);

        // A Joining FETCH is seen to its end as well.
        while more(received) || !fetches.all_over() {
            tokio::select! {
                note = fetches.next_note() => self.on_fetch_note(fetches, note)?,
                event = self.objects.next_event() => match event? {
                    UniEvent::FetchOpened { stream, header } => {
                        self.on_fetch_stream(fetches, stream, header)?;
                    }
                    UniEvent::SubgroupOpened { stream, header } => {
                        if self.belongs_to(header.track_alias, track_alias) {
                            self.open_streams.insert(stream, header.track_alias);
                        } else {
                            // §11.1, §11.4.2: a stream of an ended subscription,
                            // or of none, is not wanted.
                            report::ignored(header.track_alias);
                            self.stops.stop(stream, stream_reset_code::CANCELLED);
                        }
                    }
                    UniEvent::Object { header, object, .. } => {
                        if self.belongs_to(header.track_alias, track_alias) {
                            video::take_or_print(&mut video, &header, &object)?;
                            received += 1;
                        }
                    }
                    UniEvent::SubgroupClosed { stream, header, end } => {
                        self.open_streams.remove(&stream);
                        self.session.on_data_stream_closed(header.track_alias);
                        match end {
                            StreamEnd::Fin => {}
                            StreamEnd::Aborted(code) => report::stream_aborted(stream, code),
                            // The same line as for a reset without a code:
                            // the drivers of interop runs read it.
                            StreamEnd::Lost => report::stream_aborted(stream, None),
                        }
                    }
                    UniEvent::TrackMalformed { header, error, .. } => {
                        report::track_malformed(header.track_alias, &error);
                        let subscription_over =
                            self.cancel_malformed(fetches, header.track_alias, request_id);
                        if subscription_over {
                            self.stop_streams_of(track_alias);
                            if let Some(stream) = stream.take() {
                                stream.cancel();
                            }
                            return Err(error.into());
                        }
                    }
                    other => bail!("misrouted event {other:?}"),
                },
                event = next_request_event(&mut stream) => match event? {
                    RequestStreamEvent::Message(message) => {
                        match on_request(&mut self.session, request_id, message)? {
                            RequestEvent::SubscriptionDone(done) => {
                                report::publish_done_received(&done);
                                drain_deadline = Some(Instant::now() + DRAIN_TIMEOUT);
                            }
                            other => self.update_answered(other)?,
                        }
                    }
                    RequestStreamEvent::Closed(how) => {
                        let closed = stream.take();
                        if self.session.request_state(request_id) != Some(RequestState::Terminated) {
                            self.session.cancel(request_id);
                            self.stop_streams_of(track_alias);
                            if let Some(closed) = closed {
                                closed.cancel();
                            }
                            report::subscription_ended("request stream closed", received, wanted);
                            bail!("the peer closed the subscribe stream without PUBLISH_DONE: {how}");
                        }
                        // After PUBLISH_DONE the publisher closes the stream
                        // (§10.11). Dropping this end answers with a FIN.
                    }
                },
                () = sleep_until(drain_deadline) => {
                    self.session.on_drain_timeout(track_alias);
                    drain_deadline = None;
                    drain_timed_out = true;
                }
            }

            let done = self.session.request_state(request_id) == Some(RequestState::Terminated);
            if done && self.session.subscribed_alias(track_alias) == SubscribedAlias::Retired {
                let why = if drain_timed_out {
                    "PUBLISH_DONE, drain timeout"
                } else {
                    "PUBLISH_DONE, all streams closed"
                };
                self.stop_streams_of(track_alias);
                report::subscription_ended(why, received, wanted);
                return Ok(());
            }
        }

        // Enough Objects: the subscription is no longer wanted. Nothing happens
        // in the session when PUBLISH_DONE already ended the request.
        self.session.cancel(request_id);
        self.stop_streams_of(track_alias);
        if let Some(stream) = stream {
            stream.cancel();
        }
        Ok(())
    }

    /// Sends `STOP_SENDING` on the open subgroup streams of a subscription that
    /// is over (§10.11: "SHOULD send `STOP_SENDING` on all streams related to the
    /// subscription when it deletes subscription state").
    fn stop_streams_of(&mut self, track_alias: u64) {
        self.open_streams.retain(|stream, alias| {
            if *alias == track_alias {
                self.stops.stop(*stream, stream_reset_code::CANCELLED);
            }
            *alias != track_alias
        });
    }

    fn belongs_to(&self, track_alias: u64, subscribed: u64) -> bool {
        match self.session.subscribed_alias(track_alias) {
            // §10.11: Objects of a subscription that PUBLISH_DONE ended are still
            // its Objects while the alias drains.
            SubscribedAlias::Established(_) | SubscribedAlias::Draining(_) => {
                track_alias == subscribed
            }
            // §11.1: late Objects of an ended subscription are dropped quietly;
            // §11.4.2: so may be a stream with an alias nothing established.
            SubscribedAlias::Retired | SubscribedAlias::Unknown => false,
        }
    }

    /// Announces the namespace and offers the track. The returned streams have
    /// to stay open for as long as the track is published.
    async fn offer_track(
        &mut self,
        stdin: &mut DriverLines,
        peer_requests: &mut io::PeerRequests,
    ) -> Result<(Announced, RequestId, RequestStream)> {
        let (namespace_request, message) = self
            .session
            .publish_namespace(self.args.track_namespace()?, self.args.auth.with(vec![]))?;
        let (namespace_stream, event) = self
            .request_attending(
                peer_requests,
                &mut None,
                None,
                (namespace_request, &message, "publish-namespace"),
            )
            .await?;
        match event {
            RequestEvent::PublishNamespaceEstablished => report::namespace_published(),
            other => {
                namespace_stream.cancel();
                return Err(refused("PUBLISH_NAMESPACE", other));
            }
        }
        // The peer is answered meanwhile: a relay asks whoever announced
        // the namespace, and waits for the answer.
        self.attending(
            peer_requests,
            &mut None,
            None,
            tokio::time::sleep(self.args.publish_delay),
        )
        .await?;
        if self.args.on_line.publish {
            report::ready_to_publish();
            self.attending(
                peer_requests,
                &mut None,
                None,
                wait_for_line(stdin, "publish"),
            )
            .await??;
        }

        let (publish_request, message) = self.session.publish(
            self.args.full_track_name()?,
            PUBLISH_TRACK_ALIAS,
            self.args.auth.with(vec![Parameter::Forward(true)]),
            self.args.new_groups.track_properties()?,
        )?;
        let (publish_stream, event) = self
            .request_attending(
                peer_requests,
                &mut None,
                None,
                (publish_request, &message, "publish"),
            )
            .await?;
        match event {
            RequestEvent::PublishEstablished { forward, .. } => {
                report::published(PUBLISH_TRACK_ALIAS, forward);
            }
            other => {
                publish_stream.cancel();
                return Err(refused("PUBLISH", other));
            }
        }
        let announced = Announced {
            request: namespace_request,
            stream: Some(namespace_stream),
        };
        Ok((announced, publish_request, publish_stream))
    }

    /// Sends one Object as Object 0 of a new Group, on a new subgroup stream;
    /// with `objects` more than 1, the same as Objects 0 to `objects` - 1.
    ///
    /// A `STOP_SENDING` for the stream is not an error, whenever it comes:
    /// §11.4.3 has "A subscriber MAY send a QUIC `STOP_SENDING` frame for a
    /// subgroup stream if the Group or Subgroup is no longer of interest to
    /// it", and "Resets and `STOP_SENDING` on SUBSCRIBE data streams have no
    /// impact on other Subgroups in the Group or the subscription". A relay
    /// that takes the track from another publisher as well does it to every
    /// Subgroup it has from the other one. `SENT` is printed all the same: it
    /// says that this client did its part.
    ///
    /// Says whether a stream was opened for it: `false` where the peer allowed
    /// none in time, and the Object was not sent.
    ///
    /// WHY it borrows nothing of the client: the client answers its peer
    /// while the Object is under way ([`Self::attending`]).
    async fn send_object(
        connection: &WtConnection,
        args: &Args,
        track_alias: u64,
        group_id: u64,
        (payload, status, objects): (&[u8], ObjectStatus, u64),
    ) -> Result<bool> {
        if args.video.video_in.is_some() {
            // FIXME: a Group of `--video-in` still waits for its stream
            // without a time.
            return video::send_group(connection, track_alias, group_id, &args.video)
                .await
                .map(|()| true);
        }
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias,
            group_id,
            subgroup_id: args.subgroup_id,
            publisher_priority: args.publisher_priority,
            // §11.4.2: the bit is of the stream, and says that every Object
            // on it has the Properties Length field.
            properties: !args.object_properties.is_empty(),
            // This Object is the only one of its Group, so the stream holds the
            // Group's largest Object and its first.
            end_of_group: true,
            first_object: true,
        });
        for object_id in 0..objects {
            encoder.object(&SubgroupObject {
                object_id,
                properties: args.object_properties.clone(),
                status,
                payload: payload.to_vec(),
            })?;
        }
        // WHY a stream the peer allows no room for is not waited for, as a
        // request's is: what goes on it is a Subgroup of a live track, and
        // one that is sent late is worth less than the next. The Subgroup
        // is not sent, the caller hears it, and the session goes on.
        // `@xmoq/moqt` does the same (`StreamNotOpenedError`); a browser
        // refuses such a stream at once, and here it is given
        // `SUBGROUP_OPEN_WAIT`.
        let Ok(opened) = tokio::time::timeout(SUBGROUP_OPEN_WAIT, connection.open_uni()).await
        else {
            report::subgroup_not_sent(group_id);
            return Ok(false);
        };
        let mut data = opened?;
        let bytes = encoder.take_bytes();
        args.trace
            .bytes(Direction::Send, &format!("data#{group_id}"), &bytes);
        let failed = |error| anyhow!("could not write the subgroup stream: {error:?}");
        let Ok(written) = tokio::time::timeout(SUBGROUP_WRITE_WAIT, data.write_all(&bytes)).await
        else {
            data.reset(stream_reset_code::CANCELLED);
            report::subgroup_not_taken(group_id, SUBGROUP_WRITE_WAIT);
            return Ok(true);
        };
        match written {
            Ok(()) => {}
            // The stream was stopped before all of it was written. §11.4.3:
            // "The publisher SHOULD respond with a reset."
            Err(xmoq_moqt_io::WriteError::Stopped(code)) => {
                data.reset(stream_reset_code::CANCELLED);
                report::stop_sending_received(&format!("data#{group_id}"), code);
                report::sent(group_id, payload.len());
                return Ok(true);
            }
            Err(error) => return Err(failed(error)),
        }
        // §11.4.3: every Object of the Subgroup was written, so the stream ends
        // with a FIN.
        match tokio::time::timeout(DELIVERY_WAIT, data.finish_and_wait()).await {
            // A peer that does not want the stream (§11.4.1) has what it wants.
            Ok(Ok(())) => {}
            Ok(Err(xmoq_moqt_io::WriteError::Stopped(code))) => {
                report::stop_sending_received(&format!("data#{group_id}"), code);
            }
            Ok(Err(error)) => return Err(failed(error)),
            Err(_) => bail!("the peer did not take the subgroup stream in time"),
        }
        report::sent(group_id, payload.len());
        Ok(true)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the mode's one loop: the driver's lines, the peer's requests, and the streams of the PUBLISH_NAMESPACE and the PUBLISH"
    )]
    async fn publish(&mut self, payload: &[u8]) -> Result<()> {
        let mut stdin = BufReader::new(tokio::io::stdin()).lines();
        let mut peer_requests = self
            .peer_requests
            .take()
            .context("the readers were not started to answer requests")?;
        let (mut announced, mut publish_request, mut publish_stream) =
            self.offer_track(&mut stdin, &mut peer_requests).await?;

        // The driver writes a line for every Object it wants sent, once the
        // subscribers are ready, and closes stdin when it is done; the session
        // has to stay open in between. The request stream is read all the while: the subscriber
        // may update the subscription on it, or cancel by closing it.
        //
        // A line `republish` or `republish-same-alias` ends the subscription
        // with PUBLISH_DONE and publishes the track anew, under the next
        // Track Alias or the same one; see `Self::publish_anew`. `publish-done`
        // and then `publish-anew` or `publish-anew-same-alias` do the same in
        // two steps, with whatever the driver waits for in between.
        let track = self.args.full_track_name()?;
        let mut track_alias = PUBLISH_TRACK_ALIAS;
        // Objects sent in all, and data streams of the subscription as it is.
        let (mut sent, mut stream_count) = (0, 0);
        let status_code = loop {
            tokio::select! {
                line = stdin.next_line() => match line?.as_deref().map(PublisherLine::read) {
                    Some(PublisherLine::Republish { same_alias }) => {
                        if !same_alias {
                            track_alias += 1;
                        }
                        (publish_request, publish_stream) = self
                            .publish_anew(
                                &mut peer_requests,
                                publish_request,
                                publish_stream,
                                track_alias,
                            )
                            .await?;
                        stream_count = 0;
                    }
                    Some(PublisherLine::PublishDone) => {
                        self.end_without_a_count(publish_request, &mut publish_stream)
                            .await?;
                        report::publish_done_sent();
                        // Nothing is read from the first PUBLISH stream
                        // meanwhile: how the peer ends it is read after the
                        // new PUBLISH, as in `publish_anew`.
                        if !next_publish_anew(&mut stdin).await? {
                            track_alias += 1;
                        }
                        (publish_request, publish_stream) = self
                            .publish_after_done(
                                &mut peer_requests,
                                publish_request,
                                publish_stream,
                                track_alias,
                            )
                            .await?;
                        stream_count = 0;
                    }
                    // Each comes after the line it follows (`next_publish_anew`,
                    // `next_subgroup_again_second`), and never alone.
                    Some(line @ (PublisherLine::PublishAnew { .. } | PublisherLine::SubgroupAgainSecond)) => {
                        bail!("{line:?} without the line it follows")
                    }
                    Some(PublisherLine::SubgroupAgain { longer }) => {
                        let group = (track_alias, self.args.first_group + sent);
                        stream_count += self
                            .send_subgroup_twice((&mut peer_requests, &mut stdin), group, payload, longer)
                            .await?;
                        sent += 1;
                    }
                    Some(line @ (PublisherLine::Send | PublisherLine::EndOfTrack)) => {
                        // Each Object is a Group of its own. The Groups go on
                        // through a `republish`: it is the same track.
                        let group_id = self.args.first_group + sent;
                        let object = line.object(track_alias, group_id, payload);
                        // §10.11: the streams that were opened are counted.
                        if self.send_attending(&mut peer_requests, &mut None, None, object).await? {
                            stream_count += 1;
                        }
                        sent += 1;
                    }
                    None if sent == 0 => bail!("stdin closed before the send trigger"),
                    None => break publish_done_status::TRACK_ENDED,
                },
                turned = attention(&mut peer_requests, &mut self.answers, &mut self.objects) => {
                    self.attend(turned?, &track, &mut None, None).await?;
                }
                event = announced.next_event() => self.on_announced(&mut announced, event?)?,
                // This client has no subgroup stream open between Objects, so a
                // subscriber that ends the subscription leaves none to reset
                // (§5.1.1).
                event = publish_stream.next_event_or_stopped() => match event? {
                    RequestStreamEvent::Message(message) => {
                        match on_request(&mut self.session, publish_request, message)? {
                            // §10.9.1: a refused update also ends the
                            // subscription, with UPDATE_FAILED.
                            RequestEvent::UpdateRefused { response } => {
                                publish_stream.send(&response).await?;
                                report::update_refused();
                                break publish_done_status::UPDATE_FAILED;
                            }
                            // §10.9: the REQUEST_OK. WHY nothing else: this
                            // client acts on no delivery timeout, that of a
                            // PUBLISH_OK or a SUBSCRIBE included.
                            update @ RequestEvent::SubscriptionUpdated { .. } => {
                                answer_update(&mut publish_stream, update).await?;
                            }
                            other => bail!("unexpected message on the publish stream: {other:?}"),
                        }
                    }
                    // §5.1: the subscriber ends the subscription with
                    // STOP_SENDING. One that only finishes its direction can
                    // send no REQUEST_UPDATE any more and has ended nothing.
                    RequestStreamEvent::Closed(RequestClose::Fin) => {
                        report::subscriber_finished_its_direction();
                    }
                    RequestStreamEvent::Closed(how) => {
                        if let RequestClose::Stopped(code) = how {
                            report::stop_sending_received("request", code);
                        }
                        self.session.cancel(publish_request);
                        publish_stream.cancel();
                        bail!("the subscriber ended the subscription: {how}");
                    }
                },
            }
        };

        self.end_publishing(publish_request, publish_stream, status_code, stream_count)
            .await
    }

    /// The driver is done, or the subscription failed an update: ends the
    /// subscription of `request` with `PUBLISH_DONE` and waits for the peer
    /// to end its stream.
    async fn end_publishing(
        &mut self,
        publish_request: RequestId,
        mut publish_stream: RequestStream,
        status_code: u64,
        stream_count: u64,
    ) -> Result<()> {
        self.leave_answers();
        // §10.11: PUBLISH_DONE goes out after the data streams are closed, and
        // the request stream is closed right after it. `send_object` closes its
        // data stream before it returns.
        let done = self.session.publish_done(
            publish_request,
            status_code,
            stream_count,
            ReasonPhrase::default(),
        )?;
        publish_stream.send(&done).await?;
        let session = &mut self.session;
        let answered = publish_stream
            .finish_and_await_close(PUBLISH_DONE_ANSWER_WAIT, |message| {
                on_request(session, publish_request, message)?;
                Ok(())
            })
            .await?;
        // Whatever the peer did is reported and is not an error of this run:
        // the draft does not ask a subscriber to close its direction at all.
        report::publish_done_peer_ended(answered);
        Ok(())
    }

    /// Ends the subscription of `request` with `PUBLISH_DONE` and publishes
    /// the track anew under `track_alias`, before the peer can have seen the
    /// end of what the first subscription counted. Returns the new request
    /// and its stream.
    ///
    /// WHY the Stream Count of a publisher that cannot count (§10.11: "it
    /// MUST set Stream Count to 2^62 - 1"), though this client can: every
    /// stream it sends has ended before the `PUBLISH_DONE` goes out, so with
    /// the exact number a relay would be done with the first subscription at
    /// once. This value keeps it waiting, which is the state a publisher is
    /// in whose last stream is still under way when it publishes anew, and
    /// the one this is for.
    ///
    /// WHY there are the two steps as well (`publish-done`, then
    /// `publish-anew`): the `PUBLISH_DONE` and the PUBLISH are on two streams,
    /// between which QUIC keeps no order. A peer that reads the PUBLISH first
    /// still has the first subscription and has to refuse the second with
    /// `DUPLICATE_SUBSCRIPTION` (§5.1), and nothing tells a publisher that its
    /// `PUBLISH_DONE` was read. A driver that can see the peer read it sends
    /// the second step then; this function, in one step, can be refused.
    async fn publish_anew(
        &mut self,
        peer_requests: &mut io::PeerRequests,
        request: RequestId,
        mut stream: RequestStream,
        track_alias: u64,
    ) -> Result<(RequestId, RequestStream)> {
        self.end_without_a_count(request, &mut stream).await?;
        self.publish_after_done(peer_requests, request, stream, track_alias)
            .await
    }

    /// Sends `PUBLISH_DONE` for `request` with the Stream Count of a
    /// publisher that cannot count, and leaves its stream open.
    async fn end_without_a_count(
        &mut self,
        request: RequestId,
        stream: &mut RequestStream,
    ) -> Result<()> {
        let done = self.session.publish_done(
            request,
            publish_done_status::TRACK_ENDED,
            STREAM_COUNT_UNKNOWN,
            ReasonPhrase::default(),
        )?;
        stream.send(&done).await?;
        Ok(())
    }

    /// Publishes the track anew under `track_alias` after the `PUBLISH_DONE`
    /// of `request`, and then reports how the peer ended the stream of
    /// `request`. Returns the new request and its stream.
    async fn publish_after_done(
        &mut self,
        peer_requests: &mut io::PeerRequests,
        request: RequestId,
        stream: RequestStream,
        track_alias: u64,
    ) -> Result<(RequestId, RequestStream)> {
        // §10.2.11: LARGEST_OBJECT is there once Objects of the track were
        // published, as they were under the first PUBLISH.
        let mut parameters = vec![Parameter::Forward(true)];
        parameters.extend(self.largest.parameter());
        let parameters = self.args.auth.with(parameters);
        let (new_request, message) = self.session.publish(
            self.args.full_track_name()?,
            track_alias,
            parameters,
            vec![],
        )?;
        let (new_stream, event) = self
            .request_attending(
                peer_requests,
                &mut None,
                None,
                (new_request, &message, "publish"),
            )
            .await?;
        match event {
            RequestEvent::PublishEstablished { forward, .. } => {
                report::published(track_alias, forward);
            }
            other => {
                new_stream.cancel();
                return Err(refused("PUBLISH", other));
            }
        }
        // How the peer ends the first subscription's stream: a relay that
        // stops waiting for the stream that was counted resets it. As
        // `RequestStream::finish_and_await_close`, with the peer answered
        // meanwhile.
        let deadline = Instant::now() + PUBLISH_DONE_ANSWER_WAIT;
        let mut peers = stream.finish_own();
        let mut answered = peers.finished.then_some(RequestClose::Fin);
        while answered.is_none() {
            let next = tokio::time::timeout_at(deadline, peers.next_event());
            match self.attending(peer_requests, &mut None, None, next).await? {
                Ok(event) => match event? {
                    RequestStreamEvent::Closed(how) => answered = Some(how),
                    // What crossed the PUBLISH_DONE: the session has to see
                    // it (§10.1).
                    RequestStreamEvent::Message(message) => {
                        on_request(&mut self.session, request, message)?;
                    }
                },
                Err(_) => break,
            }
        }
        if answered.is_none() {
            peers.stop();
        }
        report::publish_done_peer_ended(answered);
        Ok((new_request, new_stream))
    }

    /// Announces the namespace and publishes the track to the peer when it
    /// subscribes. One Object goes out for every line on stdin, as in
    /// [`Self::publish`]; a line that comes while nobody is subscribed is an
    /// error of the driver, which is to wait for `SERVING`. `publish-done`
    /// ends the subscription with `PUBLISH_DONE` `TRACK_ENDED` while the
    /// namespace stays announced.
    async fn serve(&mut self, payload: &[u8]) -> Result<()> {
        let (namespace_request, message) = self
            .session
            .publish_namespace(self.args.track_namespace()?, self.args.auth.with(vec![]))?;
        let mut peer_requests = self
            .peer_requests
            .take()
            .context("the readers were not started to serve requests")?;
        let mut stdin = BufReader::new(tokio::io::stdin()).lines();
        let mut serving: Option<Serving> = None;
        let (namespace_stream, event) = self
            .request_attending(
                &mut peer_requests,
                &mut serving,
                Some(&mut stdin),
                (namespace_request, &message, "publish-namespace"),
            )
            .await?;
        match event {
            RequestEvent::PublishNamespaceEstablished => report::namespace_published(),
            other => {
                namespace_stream.cancel();
                return Err(refused("PUBLISH_NAMESPACE", other));
            }
        }
        let mut announced = Announced {
            request: namespace_request,
            stream: Some(namespace_stream),
        };
        // Each Object is a Group of its own, numbered from 0 across
        // subscriptions.
        let track = self.args.full_track_name()?;
        let mut group_id = self.args.first_group;
        loop {
            tokio::select! {
                line = stdin.next_line() => match line? {
                    // The namespace stays announced, and the next SUBSCRIBE
                    // is served.
                    Some(line) if PublisherLine::read(&line) == PublisherLine::PublishDone => {
                        self.end_served(&mut serving, publish_done_status::TRACK_ENDED)
                            .await?;
                        report::publish_done_sent();
                    }
                    Some(_) => {
                        let Some(served) = &mut serving else {
                            bail!("asked to send while nobody is subscribed");
                        };
                        served.streams += 1;
                        served.sending = true;
                        let object = (served.track_alias, group_id, payload, ObjectStatus::Normal, 1);
                        let opened = self
                            .send_attending(&mut peer_requests, &mut serving, Some(&mut stdin), object)
                            .await?;
                        if let Some(served) = serving.as_mut() {
                            served.sending = false;
                            // The stream was not opened after all, and is
                            // none of the subscription's.
                            if !opened {
                                served.streams = served.streams.saturating_sub(1);
                            }
                        }
                        // The stream is closed: a PUBLISH_DONE that waited
                        // for it goes now (an update that was refused).
                        if let Some(status) = serving.as_mut().and_then(|served| served.done_due.take()) {
                            self.end_served(&mut serving, status).await?;
                        }
                        group_id += 1;
                    }
                    None => break,
                },
                event = announced.next_event() => self.on_announced(&mut announced, event?)?,
                turned = attention(&mut peer_requests, &mut self.answers, &mut self.objects) => {
                    self.attend(turned?, &track, &mut serving, Some(&mut stdin))
                        .await?;
                }
                // §5.1: the subscriber ends the subscription with STOP_SENDING.
                // This client has no subgroup stream open between Objects, so
                // there is none to reset (§5.1.1).
                code = stopped(&mut serving) => {
                    report::stop_sending_received("request", code);
                    if let Some(ended) = serving.take() {
                        self.session.on_peer_request_closed(ended.request);
                        ended.send.reset(stream_reset_code::CANCELLED);
                        report::subscription_cancelled();
                    }
                }
            }
        }

        self.end_serving(serving, peer_requests).await
    }

    /// What came on the stream of this client's `PUBLISH_NAMESPACE` after its
    /// `REQUEST_OK`. A FIN gives nothing up: the namespace stays published
    /// (§6.2: "A subscriber can cancel the request (see Section 3.3.2) to
    /// revoke acceptance", and §3.3.2 cancels "by resetting or sending
    /// `STOP_SENDING`"; §11.4.1 ends the request with the termination of the
    /// bidirectional stream, which one direction's FIN is not). A reset or a
    /// `STOP_SENDING` withdraws it: this client's direction is reset and the
    /// mode goes on, its subscriptions with it (§6.2: "it is not a protocol
    /// error for the subscriber to send a SUBSCRIBE ... after the namespace
    /// is withdrawn").
    fn on_announced(&mut self, announced: &mut Announced, event: RequestStreamEvent) -> Result<()> {
        match event {
            RequestStreamEvent::Message(message) => {
                let event = on_request(&mut self.session, announced.request, message)?;
                bail!("unexpected on the PUBLISH_NAMESPACE stream: {event:?}");
            }
            RequestStreamEvent::Closed(RequestClose::Fin) => {}
            RequestStreamEvent::Closed(how) => {
                report::namespace_withdrawn(&how);
                self.session.cancel(announced.request);
                if let Some(stream) = announced.stream.take() {
                    stream.cancel();
                }
            }
        }
        Ok(())
    }

    /// Ends the subscription that is served, if one is, with `PUBLISH_DONE`
    /// and a FIN, and goes on: how the peer ends its direction is read as
    /// what comes on the stream of a request that is over
    /// (`said_the_last`).
    ///
    /// FIXME: the peer's direction is waited for here without an end, where
    /// [`Self::end_serving`] gives it `PUBLISH_DONE_ANSWER_WAIT`: after
    /// `UPDATE_FAILED` while the client still publishes, and after the
    /// driver's `publish-done` line. A peer that never ends it keeps the
    /// request in the session (read, not run).
    async fn end_served(&mut self, serving: &mut Option<Serving>, status: u64) -> Result<()> {
        let Some(mut ended) = serving.take() else {
            return Ok(());
        };
        let done = self.session.publish_done_on_subscribe(
            ended.request,
            status,
            ended.streams,
            ReasonPhrase::default(),
        )?;
        xmoq_moqt_io::send_message(&mut ended.send, &done, &self.args.trace, "served").await?;
        ended.send.finish();
        self.said_the_last.insert(ended.stream, ended.request);
        Ok(())
    }

    /// The driver is done: ends the subscription that is served, if one
    /// is, with `PUBLISH_DONE`, and waits for the peer to end its stream.
    async fn end_serving(
        &mut self,
        serving: Option<Serving>,
        mut peer_requests: io::PeerRequests,
    ) -> Result<()> {
        self.leave_answers();
        // The driver is done. §10.11: PUBLISH_DONE after the data streams are
        // closed, then a FIN; `send_object` closes its stream before it
        // returns.
        let Some(mut ended) = serving else {
            return Ok(());
        };
        let done = self.session.publish_done_on_subscribe(
            ended.request,
            publish_done_status::TRACK_ENDED,
            ended.streams,
            ReasonPhrase::default(),
        )?;
        xmoq_moqt_io::send_message(&mut ended.send, &done, &self.args.trace, "served").await?;
        ended.send.finish();
        // The same wait as after a PUBLISH: see `PUBLISH_DONE_ANSWER_WAIT`.
        let session = &mut self.session;
        let peer_ended = async {
            loop {
                match peer_requests.next_event().await? {
                    PeerRequestStreamEvent::Closed { stream, how } if stream == ended.stream => {
                        return Ok::<_, anyhow::Error>(how.is_fin());
                    }
                    // What crossed the PUBLISH_DONE is ignored, but the
                    // session has to see it: a REQUEST_UPDATE uses up a
                    // Request ID (§10.1).
                    PeerRequestStreamEvent::Message { stream, message }
                        if stream == ended.stream =>
                    {
                        session.on_peer_request_message(ended.request, message)?;
                    }
                    // Bytes that do not decode close the session with the
                    // code of that error, as after a PUBLISH.
                    PeerRequestStreamEvent::Failed(error) => return Err(error.into()),
                    PeerRequestStreamEvent::ReaderPanicked => {
                        return Err(client::ReaderPanicked.into());
                    }
                    // The driver is done: other requests are not served any
                    // more.
                    PeerRequestStreamEvent::Opened { .. }
                    | PeerRequestStreamEvent::Message { .. }
                    | PeerRequestStreamEvent::Closed { .. } => {}
                }
            }
        };
        let fin = match tokio::time::timeout(PUBLISH_DONE_ANSWER_WAIT, peer_ended).await {
            Ok(fin) => Some(fin?),
            Err(_) => None,
        };
        self.session.on_peer_request_closed(ended.request);
        report::publish_done_peer_ended_on_its_stream(fin);
        Ok(())
    }

    /// What the flags put between a SUBSCRIBE of the peer and its
    /// `SUBSCRIBE_OK`: a time, and a line of the driver.
    ///
    /// The peer is left unanswered in two waits of a mode that publishes:
    /// this one, and the up to 3 seconds after the last `PUBLISH_DONE`, when
    /// the client waits for the peer to end that stream. A request of the
    /// peer that comes in the first is answered when the wait is over; one
    /// that comes in the second gets no answer, and the session closes right
    /// after. In every other wait (the answers to its `PUBLISH_NAMESPACE` and
    /// PUBLISH, `--publish-delay-ms`, the driver's line `publish`, an Object
    /// under way, the end of the first stream when it publishes anew) the
    /// client answers ([`Self::attending`]). What it holds unread meanwhile is
    /// `EVENT_QUEUE` events a queue (`src/io.rs`), after which its readers
    /// wait.
    async fn before_serving(&self, request: PeerRequestId, stdin: &mut DriverLines) -> Result<()> {
        tokio::time::sleep(self.args.serve_delay).await;
        if self.args.on_line.serve {
            report::asked(request.value());
            wait_for_line(stdin, "serve").await?;
        }
        Ok(())
    }

    /// Waits for `work`, and meanwhile turns to what a client that publishes
    /// has to: the requests of the peer, the FETCH requests it answers, the
    /// streams the peer opens. `serving` and `stdin` are those of the mode
    /// that serves a SUBSCRIBE; without `stdin` none is served meanwhile.
    ///
    /// WHY every wait of a mode that publishes goes through here: a relay
    /// asks whoever announced a namespace and waits for the answer, whatever
    /// this client is waiting for itself.
    async fn attending<T>(
        &mut self,
        peer_requests: &mut io::PeerRequests,
        serving: &mut Option<Serving>,
        mut stdin: Option<&mut DriverLines>,
        work: impl Future<Output = T>,
    ) -> Result<T> {
        let track = self.args.full_track_name()?;
        tokio::pin!(work);
        loop {
            let turned = tokio::select! {
                done = &mut work => return Ok(done),
                turned = attention(peer_requests, &mut self.answers, &mut self.objects) => turned?,
            };
            self.attend(turned, &track, serving, stdin.as_deref_mut())
                .await?;
        }
    }

    /// Notes an Object (its Track Alias, its Group, its payload and its
    /// status; one that is not `Normal` has no payload, §2.1) as published
    /// and sends it with [`Self::send_object`], answering the peer while it
    /// is under way.
    ///
    /// WHY noted before it is sent: §5.1.2 has "Largest Object updates when
    /// the first byte of an Object with a Location larger than the previous
    /// value is published", and the relay takes the Object for the largest
    /// from its first byte as well. A FETCH or a SUBSCRIBE that this client answers
    /// while the Object is under way would otherwise be answered as if the
    /// Object did not exist: a Standalone FETCH (`--answer-fetch`) would end
    /// before it, and the subscription begin after it. An Object that could not be written
    /// ends the mode, and the session with it.
    async fn send_attending(
        &mut self,
        peer_requests: &mut io::PeerRequests,
        serving: &mut Option<Serving>,
        stdin: Option<&mut DriverLines>,
        (track_alias, group_id, payload, status, objects): (u64, u64, &[u8], ObjectStatus, u64),
    ) -> Result<bool> {
        self.sent(group_id, payload);
        let sending = Self::send_object(
            self.connection,
            self.args,
            track_alias,
            group_id,
            (payload, status, objects),
        );
        self.attending(peer_requests, serving, stdin, sending)
            .await?
    }

    /// `subgroup-again-longer` and `subgroup-again-shorter`
    /// ([`PublisherLine::SubgroupAgain`]): the Group on two streams one
    /// after the other, the second at the driver's `subgroup-again-second`.
    /// Returns how many streams were opened.
    async fn send_subgroup_twice(
        &mut self,
        (peer_requests, stdin): (&mut io::PeerRequests, &mut DriverLines),
        (track_alias, group_id): (u64, u64),
        payload: &[u8],
        longer: bool,
    ) -> Result<u64> {
        let (first, again) = if longer { (1, 2) } else { (2, 1) };
        let mut opened = 0;
        // WHY the second waits for a line of the driver, and not only for the
        // FIN of the first, which `send_object` waits for: QUIC keeps no
        // order between two streams, so a second stream opened after the
        // first's FIN was written can still reach the relay first. Then the
        // relay holds the second as the Subgroup's stream and ends the first
        // (§11.4), and what the run looks for, a Subgroup delivered whole and
        // then sent again, did not happen: in a run of CI (2026-10-25) the
        // short stream came first and was reset with 0x1. The driver writes
        // the line once the relay has finished its streams of the first
        // towards every subscriber (`subgroupAgain` of `interop/disconnect.ts`).
        for (index, objects) in [first, again].into_iter().enumerate() {
            if index == 1 {
                next_subgroup_again_second(stdin).await?;
            }
            let object = (
                track_alias,
                group_id,
                payload,
                ObjectStatus::Normal,
                objects,
            );
            if self
                .send_attending(peer_requests, &mut None, None, object)
                .await?
            {
                opened += 1;
            }
        }
        Ok(opened)
    }

    /// Sends a request on a new stream and returns the event its response
    /// produces, answering the peer while the response is waited for. On any
    /// failure the request is cancelled, in the session and on the stream, so
    /// it does not stay open on either. `request` is the request, its message and the label of its
    /// stream.
    async fn request_attending(
        &mut self,
        peer_requests: &mut io::PeerRequests,
        serving: &mut Option<Serving>,
        stdin: Option<&mut DriverLines>,
        request: (RequestId, &ControlMessage, &str),
    ) -> Result<(RequestStream, RequestEvent)> {
        let (request_id, message, label) = request;
        let mut stream = self.open_request(request_id, message, label).await?;
        let wait =
            xmoq_moqt::params::answer_wait_ms(self.args.waits.answer, parameters_of(message));
        let answer = tokio::time::timeout(Duration::from_millis(wait), stream.next_event());
        let Ok(event) = self
            .attending(peer_requests, serving, stdin, answer)
            .await?
        else {
            let error = not_answered(label, wait);
            if matches!(message, ControlMessage::Publish(_)) {
                self.end_unanswered_publish(request_id, stream).await?;
            } else {
                self.session.cancel(request_id);
                stream.cancel();
            }
            return Err(error);
        };
        match event.and_then(|event| self.on_response(request_id, event, label)) {
            Ok(event) => Ok((stream, event)),
            Err(error) => {
                self.session.cancel(request_id);
                stream.cancel();
                Err(error)
            }
        }
    }

    /// Ends a PUBLISH that got no answer in time: `PUBLISH_DONE` and a FIN,
    /// and the subscriber has the time it has after any `PUBLISH_DONE` to
    /// end its direction; then that is stopped.
    ///
    /// WHY `PUBLISH_DONE`, where every other request is reset: a PUBLISH
    /// that waits for its answer is a subscription in the state Pending
    /// (Publisher), and §5.1 has "The publisher terminates a subscription
    /// in the Pending (Publisher) or Established states by sending
    /// `PUBLISH_DONE` and closing the stream"; §11.4.1: "When possible,
    /// Publishers SHOULD send a `PUBLISH_DONE` when terminating a
    /// subscription instead of abruptly terminating the associated control
    /// stream".
    /// WHY `INTERNAL_ERROR`: §10.11 has no code for a time that ran out
    /// (`EXPIRED` is "the timeout specified in `SUBSCRIBE_OK`"), and this
    /// one is "An implementation specific or generic error". No stream was
    /// opened, so Stream Count is 0. `@xmoq/moqt` sends the same
    /// (`endUnansweredPublish`).
    async fn end_unanswered_publish(
        &mut self,
        request_id: RequestId,
        mut stream: RequestStream,
    ) -> Result<()> {
        let done = self.session.publish_done(
            request_id,
            publish_done_status::INTERNAL_ERROR,
            0,
            ReasonPhrase::new(NO_ANSWER_IN_TIME)?,
        )?;
        if stream.send(&done).await.is_err() {
            stream.cancel();
            return Ok(());
        }
        // An answer that crossed the PUBLISH_DONE is the peer's to send.
        stream
            .finish_and_await_close(PUBLISH_DONE_ANSWER_WAIT, |_| Ok(()))
            .await?;
        Ok(())
    }

    /// Turns to one thing. `stdin` is there in the mode that serves a
    /// SUBSCRIBE, which may wait for a line of the driver before it does.
    async fn attend(
        &mut self,
        turned: Attention,
        track: &FullTrackName,
        serving: &mut Option<Serving>,
        stdin: Option<&mut DriverLines>,
    ) -> Result<()> {
        match turned {
            Attention::Peer(event) => {
                self.on_peer_request_event(event, track, serving, stdin)
                    .await
            }
            Attention::Note(answers::Note(stream, happened)) => {
                if let Some(over) = self.answers.happened(stream, happened) {
                    self.fetch_over(stream, over);
                }
                Ok(())
            }
            Attention::Data(event) => self.on_unasked_stream(event),
        }
    }

    /// A unidirectional stream of the peer, in a client that publishes: it
    /// subscribed to nothing and sent no FETCH.
    fn on_unasked_stream(&mut self, event: UniEvent) -> Result<()> {
        match event {
            // The session says what the stream belongs to: one for a
            // request of this client that is over is stopped, any other
            // closes the session (`Session::on_fetch_stream`).
            UniEvent::FetchOpened { stream, header } => {
                self.session.on_fetch_stream(header.request_id)?;
                // The body, dropped, stops the stream.
                drop(self.stops.take_fetch_body(stream));
            }
            // §11.1, §11.4.2: a stream of no subscription is not wanted.
            UniEvent::SubgroupOpened { stream, header } => {
                report::ignored(header.track_alias);
                self.stops.stop(stream, stream_reset_code::CANCELLED);
            }
            // What was read of such a stream before it was stopped.
            UniEvent::Object { .. }
            | UniEvent::SubgroupClosed { .. }
            | UniEvent::TrackMalformed { .. } => {}
            other => bail!("misrouted event {other:?}"),
        }
        Ok(())
    }

    /// Notes an Object that was sent, Object 0 of its Group, and keeps it
    /// for a FETCH where this client answers one.
    fn sent(&mut self, group_id: u64, payload: &[u8]) {
        self.largest.sent(group_id);
        if self.args.fetch.answering.is_none() {
            return;
        }
        self.published.push(FetchObject {
            group_id,
            // Every `--subgroup-id` gives the one Object of a Group, Object
            // 0, the Subgroup 0.
            subgroup_id: Some(0),
            object_id: 0,
            // This client sends no Track Property, so the track has the
            // default of §12.4.
            publisher_priority: self
                .args
                .publisher_priority
                .unwrap_or(track_property::DEFAULT_PUBLISHER_PRIORITY_IF_OMITTED),
            properties: self.args.object_properties.clone(),
            payload: payload.to_vec(),
        });
    }

    /// A FETCH this client answered is over, or a step after that.
    fn fetch_over(&mut self, stream: u64, over: Conclusion) {
        if let Some(served) = over.served {
            report::fetch_served(served.request_id, served.how, served.code, served.objects);
        }
        answers::conclude(
            &over,
            stream,
            &mut self.session,
            &mut self.said_the_last,
            &self.stops,
        );
    }

    /// The mode is done: the FETCH requests that are still answered end.
    fn leave_answers(&mut self) {
        for over in self.answers.leave() {
            if let Some(served) = over.served {
                report::fetch_served(served.request_id, served.how, served.code, served.objects);
            }
            self.session.on_peer_request_closed(over.request);
        }
    }

    /// Sends the last this client says on a request stream of the peer, and
    /// a FIN, after `--fetch-answer-delay-ms`.
    ///
    /// WHY a task: the delay is there to keep a relay waiting, and the
    /// client goes on with its mode meanwhile.
    fn answer_later(
        &mut self,
        mut send: <WtConnection as Connection>::Send,
        response: ControlMessage,
        label: String,
    ) {
        let (delay, trace) = (self.args.answer_delay, self.args.trace.clone());
        // Those that are done are let go of, so that the set holds what
        // still waits and no more.
        while self.later.try_join_next().is_some() {}
        self.later.spawn(async move {
            tokio::time::sleep(delay).await;
            // An error means the peer is gone or stopped the stream.
            let _ = xmoq_moqt_io::send_message(&mut send, &response, &trace, &label).await;
            send.finish();
        });
    }

    /// What a FETCH of the peer gets from the Objects this client sent
    /// (§10.12.3), and the Group Order it asked for; or the code to refuse
    /// it with.
    fn plan_fetch(
        &self,
        request: PeerRequestId,
        fetch: &Fetch,
        track: &FullTrackName,
    ) -> Result<(published::Answer, GroupOrder), u64> {
        // A relay turns a Joining FETCH into a Standalone one before it
        // asks a publisher, as `@xmoq/moqt` assumes too.
        let FetchRange::Standalone {
            full_track_name,
            start,
            end,
        } = &fetch.range
        else {
            return Err(request_error_code::NOT_SUPPORTED);
        };
        if full_track_name != track {
            return Err(request_error_code::DOES_NOT_EXIST);
        }
        report::fetch_asked(request.value(), *start, *end);
        let group_order = answers::group_order(fetch);
        let answer = if self.answers.has_room() {
            self.published.answer(*start, *end, group_order)
        } else {
            Err(request_error_code::EXCESSIVE_LOAD)
        };
        let answer = answer.and_then(|answer| {
            if self.answers.fits(&answer.pieces) {
                Ok(answer)
            } else {
                Err(request_error_code::EXCESSIVE_LOAD)
            }
        });
        match answer {
            Ok(answer) => Ok((answer, group_order)),
            Err(code) => {
                report::fetch_not_answered(request.value(), code);
                Err(code)
            }
        }
    }

    /// Accepts a FETCH of the peer and has it answered on its streams.
    fn start_fetch(
        &mut self,
        stream: u64,
        request: PeerRequestId,
        send: <WtConnection as Connection>::Send,
        (answer, group_order): (published::Answer, GroupOrder),
    ) -> Result<()> {
        // End Of Track 0: the track goes on while this client answers.
        let messages = answers::accepted(&mut self.session, request, answer.end_location)?;
        report::fetch_answered(request.value(), answer.end_location);
        self.answers.start(
            stream,
            request,
            send,
            Answer {
                messages,
                header: FetchHeader {
                    request_id: request.value(),
                },
                group_order,
                pieces: answer.pieces,
            },
            self.connection,
            &self.args.trace,
        );
        Ok(())
    }

    /// A request of the peer, in the modes that publish.
    async fn on_peer_request_opened(
        &mut self,
        stream: u64,
        mut send: <WtConnection as Connection>::Send,
        first: ControlMessage,
        track: &FullTrackName,
        serving: &mut Option<Serving>,
        stdin: Option<&mut DriverLines>,
    ) -> Result<()> {
        let wire = &self.args.trace;
        let label = format!("bi#{stream}");
        let (request, peer_request) = match self.session.on_peer_request_opened(first)? {
            PeerRequestOpened::Request(request, peer_request) => (request, peer_request),
            PeerRequestOpened::Refused { response, request } => {
                self.answer_later(send, response, label);
                if let Some(request) = request {
                    self.said_the_last.insert(stream, request);
                }
                return Ok(());
            }
        };
        let wanted = matches!(
            &peer_request,
            PeerRequest::Subscribe(subscribe) if subscribe.full_track_name == *track
        );
        if wanted
            && serving.is_none()
            && let Some(stdin) = stdin
        {
            self.before_serving(request, stdin).await?;
            // §10.2.11: there once this client has published an
            // Object of the track.
            let largest = self.largest.parameter().into_iter().collect();
            let (track_alias, ok) = self.session.accept_subscribe(request, largest, vec![])?;
            xmoq_moqt_io::send_message(&mut send, &ok, wire, &label).await?;
            report::serving(track_alias);
            *serving = Some(Serving {
                request,
                stream,
                send,
                track_alias,
                streams: 0,
                sending: false,
                done_due: None,
            });
            return Ok(());
        }
        // Another track, or a kind of request this client does not
        // serve. A second SUBSCRIBE for the track while one is served
        // does not get here: the session refuses it itself
        // (DUPLICATE_SUBSCRIPTION, §5.1).
        let serves = matches!(self.args.mode, Mode::PublishNamespace { .. });
        let error_code = match fetch_to_answer(self.args.fetch.answering, &peer_request) {
            Some(fetch) => match self.plan_fetch(request, fetch, track) {
                Ok(plan) => return self.start_fetch(stream, request, send, plan),
                Err(code) => code,
            },
            None => refusal_code(serves, &peer_request),
        };
        // EXCESSIVE_LOAD with a Retry Interval: room comes back once other
        // answers are written.
        let retry = retry_interval::made_up(error_code);
        let response = self
            .session
            .reject(request, error_code, retry, ReasonPhrase::default())?;
        self.answer_later(send, response, label);
        // The session keeps a request that was rejected until its
        // stream is gone: what still arrives on it is given to the
        // session (below).
        self.said_the_last.insert(stream, request);
        Ok(())
    }

    /// One event of a request stream the peer opened, in the modes that
    /// publish.
    async fn on_peer_request_event(
        &mut self,
        event: PeerRequestStreamEvent<<WtConnection as Connection>::Send>,
        track: &FullTrackName,
        serving: &mut Option<Serving>,
        stdin: Option<&mut DriverLines>,
    ) -> Result<()> {
        let wire = &self.args.trace;
        match event {
            PeerRequestStreamEvent::Opened {
                stream,
                send,
                first,
            } => {
                self.on_peer_request_opened(stream, send, first, track, serving, stdin)
                    .await?;
            }
            PeerRequestStreamEvent::Message { stream, message } => {
                if let Some(request) = self.answers.request_on(stream) {
                    // A REQUEST_UPDATE for a FETCH that is answered changes
                    // nothing and gets its REQUEST_OK.
                    let event = self.session.on_peer_request_message(request, message)?;
                    if let PeerRequestEvent::UpdateAccepted { response, .. } = event
                        && let Some(over) = self.answers.say(stream, response)
                    {
                        self.fetch_over(stream, over);
                    }
                    return Ok(());
                }
                let Some(served) = serving.as_mut().filter(|served| served.stream == stream) else {
                    // What crosses this client's PUBLISH_DONE or its
                    // REQUEST_ERROR is ignored by the session, which still
                    // has to see it.
                    on_refused_message(&mut self.session, &self.said_the_last, stream, message)?;
                    return Ok(());
                };
                let event = self
                    .session
                    .on_peer_request_message(served.request, message)?;
                // §10.9: an update of the subgroup timeout, or of nothing,
                // gets its REQUEST_OK; this client acts on no delivery
                // timeout.
                if let PeerRequestEvent::SubscriptionUpdated { response, .. } = &event {
                    xmoq_moqt_io::send_message(&mut served.send, response, wire, "served").await?;
                }
                // Any other update is refused: the first one is answered
                // with REQUEST_ERROR, and the subscription ends (§10.9.1),
                // with its PUBLISH_DONE once the stream being written, if
                // one is, is closed (§10.11).
                if let PeerRequestEvent::UpdateRefused { response, .. } = event {
                    xmoq_moqt_io::send_message(&mut served.send, &response, wire, "served").await?;
                    report::update_refused();
                    if served.sending {
                        served.done_due = Some(publish_done_status::UPDATE_FAILED);
                    } else {
                        self.end_served(serving, publish_done_status::UPDATE_FAILED)
                            .await?;
                    }
                }
            }
            PeerRequestStreamEvent::Closed { stream, how } => {
                let closed = answers::Happened::PeerClosed(how);
                if let Some(over) = self.answers.happened(stream, closed) {
                    self.fetch_over(stream, over);
                }
                on_refused_closed(&mut self.session, &mut self.said_the_last, stream);
                // A subscriber that only finishes its direction has not ended
                // the subscription (§5.1); a reset has (§3.3.2).
                if !how.is_fin()
                    && serving
                        .as_ref()
                        .is_some_and(|served| served.stream == stream)
                    && let Some(ended) = serving.take()
                {
                    self.session.on_peer_request_closed(ended.request);
                    ended.send.reset(stream_reset_code::CANCELLED);
                    report::subscription_cancelled();
                }
            }
            PeerRequestStreamEvent::Failed(error) => return Err(error.into()),
            PeerRequestStreamEvent::ReaderPanicked => return Err(client::ReaderPanicked.into()),
        }
        Ok(())
    }

    async fn run_mode(&mut self) -> Result<()> {
        match &self.args.mode {
            Mode::Subscribe { objects } => {
                if self.args.on_line.subscribe {
                    report::ready_to_subscribe();
                    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
                    // Nothing on a data stream is asked for before the
                    // SUBSCRIBE (`response_stopping_data`).
                    loop {
                        tokio::select! {
                            line = wait_for_line(&mut stdin, "subscribe") => break line?,
                            event = self.objects.next_event() => self.on_unasked_stream(event?)?,
                        }
                    }
                }
                self.subscribe(*objects).await?;
                if !self.args.video.stay_after.is_zero() {
                    report::staying();
                    tokio::time::sleep(self.args.video.stay_after).await;
                }
                Ok(())
            }
            Mode::Fetch { start, end } => self.fetch(*start, *end).await,
            Mode::Publish { payload } => self.publish(payload).await,
            Mode::PublishNamespace { payload } => self.serve(payload).await,
            Mode::SubscribeNamespace => self.watch_namespaces().await,
            Mode::TrackStatus => self.track_status().await,
        }
    }
}

/// What has to stay until the session is closed, so the caller keeps it.
#[derive(Default)]
struct UntilClosed {
    /// This endpoint's control stream. §3.3: it stays open for the whole
    /// session; ending it earlier is a `PROTOCOL_VIOLATION` for the peer.
    control: Option<<WtConnection as Connection>::Send>,
    /// The readers of the peer's streams, its control stream among them.
    /// WHY kept: letting go of a stream that is not over tells the peer to
    /// stop sending on it, and the close of the session takes a moment to
    /// reach the peer. A relay that sees its control stream stopped first
    /// closes the session with `PROTOCOL_VIOLATION`.
    readers: Option<io::Readers>,
}

/// Runs the session until its mode is done. `kept` receives what has to
/// outlive this function.
async fn run_session(
    connection: &Arc<WtConnection>,
    kept: &mut UntilClosed,
    args: &Args,
) -> Result<()> {
    let mut session = Session::new(Role::Client, Transport::WebTransport);
    let stops = StreamStops::default();
    // The modes that publish ask their session about every request of the
    // peer; the others answer none.
    let serve = matches!(
        args.mode,
        Mode::Publish { .. } | Mode::PublishNamespace { .. }
    );
    let (mut session_inbox, mut objects, peer_requests, readers) = io::start_readers(
        Arc::clone(connection),
        args.trace.clone(),
        stops.clone(),
        serve,
        args.answer_delay,
        args.waits.idle_subgroup_stream,
    );
    kept.readers = Some(readers);

    // §3.3: each peer opens one control stream that begins with SETUP.
    // WHY NOT `client::setup`: this client's readers are its own
    // (`io::start_readers`, with `--trace` and `--fetch-answer-delay-ms`), and hand
    // the control messages on through `session_inbox`, not a channel of
    // `UniEvent`; and its SETUP is traced.
    let control = kept.control.insert(connection.open_uni().await?);
    let implementation = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
    let setup_options = client::setup_options(args.auth.token(), implementation)?;
    let setup = session.setup(setup_options)?;
    xmoq_moqt_io::send_message(control, &setup, &args.trace, "control").await?;

    let setup_wait = Duration::from_millis(args.waits.setup);
    let waited = io::first_control_message(&mut session_inbox, &mut objects);
    let Ok(first) = tokio::time::timeout(setup_wait, waited).await else {
        return Err(client::NoSetupInTime(setup_wait).into());
    };
    let ControlEvent::Setup(peer_setup) = session.on_control_message(first?)? else {
        bail!("the session took a first control message that is not SETUP");
    };
    report::setup_ok(&peer_setup);

    let mut client = Client {
        connection,
        session,
        objects,
        stops,
        open_streams: HashMap::new(),
        peer_requests,
        said_the_last: HashMap::new(),
        update_answer: None,
        largest: Largest::default(),
        later: tokio::task::JoinSet::new(),
        published: Published::default(),
        answers: Answers::new(
            args.fetch.answering.unwrap_or_default(),
            Bounds {
                at_once: MAX_ANSWERED_AT_ONCE,
                bytes: MAX_BYTES_HELD,
                say_queue: SAY_QUEUE_OF_A_PAGE,
            },
        ),
        args,
    };
    // The session inbox is watched for the rest of the session: it reports a
    // closed control stream, a failed stream reader and any further control
    // message, each of which ends the session.
    let later = tokio::select! {
        result = client.run_mode() => match result {
            Err(error) if error.chain().any(<dyn std::error::Error>::is::<RequestGoneAway>) => {
                if let Some(RequestGoneAway(go_away)) =
                    error.chain().find_map(|cause| cause.downcast_ref())
                {
                    report::request_go_away(go_away);
                }
                return Ok(());
            }
            // The GOAWAY behind the refusal, which travels on another
            // stream and may be read after it. WHY a limit, the time an
            // answer gets: GOING_AWAY also comes from a peer that received a
            // GOAWAY (§10.6.2) and sends none. `@xmoq/moqt` waits the same
            // (`goAwayBehind`, `packages/moqt/src/webtransport/keep-moving.ts`).
            Err(error) if error.chain().any(<dyn std::error::Error>::is::<GoingAway>) => {
                let wait = Duration::from_millis(client.args.waits.answer);
                match tokio::time::timeout(wait, session_inbox.next_control_message()).await {
                    Ok(event) => event?,
                    Err(_) => return Err(error),
                }
            }
            other => return other,
        },
        event = session_inbox.next_control_message() => event?,
    };
    // A GOAWAY ends the session with NO_ERROR, and its mode with it; any
    // other message is a protocol error to close with. WHY NOT go on with
    // the mode, which §3.6 recommends ("waits until there are no more
    // Established subscriptions before closing"): the mode holds the
    // session while it runs, and the GOAWAY can be taken only once it has
    // been dropped. This client connects to one URL and does not move.
    match client.session.on_control_message(later)? {
        ControlEvent::GoAway(go_away) => {
            report::go_away(&go_away);
            Ok(())
        }
        ControlEvent::Setup(_) => bail!("the session accepted a second SETUP"),
    }
}

/// The code to close the session with (§3.5), `NO_ERROR` for a normal end.
fn termination_code(result: &Result<()>) -> u32 {
    client::termination_code_of(result)
}

/// How long a subgroup stream may wait to be opened: for the peer to allow
/// one more. **Provisional**: long against the moment a stream takes to be
/// opened where there is room, short against a Group of a live track.
const SUBGROUP_OPEN_WAIT: Duration = Duration::from_millis(100);

/// How long a request may wait for its stream to be opened. **Provisional**,
/// and the time `@xmoq/moqt` gives it (`DEFAULT_OPEN_STREAM_TIMEOUT_MS`).
const STREAM_OPEN_WAIT_MS: u64 = 10_000;

/// `Session::on_request_message`, where a GOAWAY on the request's stream
/// (§10.4) ends the mode as a GOAWAY on the control stream does: this client
/// does not send the request again, and leaves with `NO_ERROR`.
fn on_request(
    session: &mut Session,
    request: RequestId,
    message: ControlMessage,
) -> Result<RequestEvent> {
    match session.on_request_message(request, message)? {
        RequestEvent::GoAway(go_away) => Err(RequestGoneAway(go_away).into()),
        event => Ok(event),
    }
}

/// A GOAWAY on the stream of a request of this client.
#[derive(Debug)]
struct RequestGoneAway(GoAway);

impl std::fmt::Display for RequestGoneAway {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the peer sent GOAWAY on a request stream")
    }
}

impl std::error::Error for RequestGoneAway {}

/// A request refused with `GOING_AWAY` (§10.4): the peer has sent GOAWAY,
/// which may come after the refusal on the control stream.
#[derive(Debug)]
struct GoingAway(String);

impl std::fmt::Display for GoingAway {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} rejected: GOING_AWAY", self.0)
    }
}

impl std::error::Error for GoingAway {}

/// How long a client that failed waits to learn whether the peer closed the
/// session, and with what ([`run`]).
const PEER_CLOSE_WAIT: Duration = Duration::from_millis(500);

async fn run(args: &Args) -> Result<()> {
    // WHY one deadline for connecting and the session: `--timeout-secs` bounds
    // the whole run, and a relay that never answers the handshake must not
    // outlast it.
    let deadline = Instant::now() + Duration::from_secs(args.timeout_secs);
    let timed_out = || anyhow!("timed out after {}s", args.timeout_secs);

    let connection = tokio::time::timeout_at(
        deadline,
        wt::connect(&args.url, args.cert_sha256, &StreamConfig::default()),
    )
    .await
    .map_err(|_| timed_out())??;
    let connection = Arc::new(connection);
    report::connected(&args.url);

    // WHY the timeout sits here and not around `run`: the close below has to
    // happen on every path, a timeout included.
    let mut kept = UntilClosed::default();
    let session = run_session(&connection, &mut kept, args);
    let result = match tokio::time::timeout_at(deadline, session).await {
        Ok(result) => result,
        Err(_) => Err(timed_out()),
    };
    // A session the peer closed: what failed first in this client is one of
    // several things (a stream that ended, a read that was lost), and none
    // of them says why. The connection does, a moment later.
    if result.is_err()
        && let Ok(ConnectionEnd::ClosedByPeer { code, reason }) =
            tokio::time::timeout(PEER_CLOSE_WAIT, connection.closed()).await
    {
        report::closed_by_peer(code, &reason);
    }
    connection.close(termination_code(&result), b"");
    // The close carries its code to the peer, which takes a moment; a process
    // that exits before that leaves the peer with a lost connection.
    let _ = tokio::time::timeout(CLOSE_GRACE, connection.closed()).await;
    drop(kept);
    result
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Arc::new(parse_args(std::env::args().skip(1))?);
    if args.video.viewers == 0 {
        return run(&args).await;
    }
    // `--viewers`: every session is a client of its own, as if it were a
    // process.
    let mut sessions = tokio::task::JoinSet::new();
    for _ in 0..args.video.viewers {
        let args = Arc::clone(&args);
        sessions.spawn(async move { run(&args).await });
        // WHY not all at once: hundreds of handshakes in one instant are a
        // load of their own, and not the one that is measured.
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (mut ended, mut failed) = (0, 0);
    while let Some(result) = sessions.join_next().await {
        match result {
            Ok(Ok(())) => ended += 1,
            Ok(Err(error)) => {
                failed += 1;
                println!("VIEWER_FAILED {error:#}");
            }
            Err(error) => {
                failed += 1;
                println!("VIEWER_FAILED {error}");
            }
        }
    }
    println!("VIEWERS_ENDED ok={ended} failed={failed}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xmoq_moqt::message::{
        Publish, PublishDone, PublishNamespace, Subscribe, SubscribeNamespace,
    };

    #[test]
    fn the_first_group_is_a_number_or_the_time() {
        assert_eq!(first_group_of("0").unwrap(), 0);
        assert_eq!(first_group_of("1000").unwrap(), 1_000);
        // The milliseconds since 1970: more than those of 2026-01-01, and
        // not less the second time.
        let now = first_group_of("now").unwrap();
        assert!(now > 1_767_225_600_000, "{now}");
        assert!(first_group_of("now").unwrap() >= now);
        assert!(first_group_of("soon").is_err());
        assert!(first_group_of("-1").is_err());
    }

    #[test]
    fn the_idle_time_of_a_subgroup_stream_is_taken_from_1_ms_and_never_0() {
        // (the value of --idle-subgroup-stream-ms; the time, or None where it
        //  is refused)
        let cases: [(&str, Option<Duration>); 5] = [
            ("1", Some(Duration::from_millis(1))),
            ("1500", Some(Duration::from_millis(1_500))),
            ("2147483647", Some(Duration::from_millis(2_147_483_647))),
            ("0", None),
            ("2147483648", None),
        ];
        for (value, expected) in cases {
            let mut waits = Waits::default();
            let read = waits.read("--idle-subgroup-stream-ms", value);
            assert_eq!(
                read.ok().map(|_| waits.idle_subgroup_stream.get()),
                expected,
                "{value}"
            );
        }
        assert_eq!(
            Waits::default().idle_subgroup_stream.get(),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn the_session_is_closed_with_the_code_its_failure_asks_for() {
        // (how the mode ended; the code the session is closed with)
        let cases: [(Result<()>, u32); 4] = [
            (Ok(()), 0x0),
            (Err(anyhow!("the peer did not answer")), 0x0),
            (Err(client::NoSetupInTime(client::SETUP_WAIT).into()), 0x11),
            (
                Err(
                    anyhow::Error::from(client::NoSetupInTime(client::SETUP_WAIT))
                        .context("the session"),
                ),
                0x11,
            ),
        ];
        for (result, code) in cases {
            assert_eq!(termination_code(&result), code, "{result:?}");
        }
    }

    /// `fixtures/request-answer-wait/`, which `@xmoq/moqt` reads too: what
    /// this client ends a PUBLISH with that nobody answered.
    #[test]
    fn the_publish_done_of_an_unanswered_publish_is_the_shared_one() {
        let shared: Vec<u8> = include_str!(
            "../../../fixtures/request-answer-wait/publish-done-of-an-unanswered-publish.hex"
        )
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect();
        let mut sent = Vec::new();
        ControlMessage::PublishDone(PublishDone {
            status_code: publish_done_status::INTERNAL_ERROR,
            stream_count: 0,
            error_reason: ReasonPhrase::new(NO_ANSWER_IN_TIME).unwrap(),
        })
        .encode(&mut sent)
        .unwrap();
        assert_eq!(sent, shared);
    }

    #[test]
    fn a_request_this_client_does_not_serve_is_refused_with_a_code_of_its_kind() {
        let track = FullTrackName::new(
            TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap(),
            b"a".to_vec(),
        )
        .unwrap();
        let subscribe = PeerRequest::Subscribe(Subscribe {
            request_id: 1,
            full_track_name: track.clone(),
            parameters: vec![],
        });
        let publish = PeerRequest::Publish(Publish {
            request_id: 1,
            full_track_name: track.clone(),
            track_alias: 1,
            parameters: vec![],
            track_properties: vec![],
        });
        let publish_namespace = PeerRequest::PublishNamespace(PublishNamespace {
            request_id: 1,
            track_namespace: track.namespace().clone(),
            parameters: vec![],
        });
        let fetch = PeerRequest::Fetch(Fetch {
            request_id: 1,
            range: FetchRange::Standalone {
                full_track_name: track.clone(),
                start: at(0, 0),
                end: at(1, 0),
            },
            parameters: vec![],
        });
        let subscribe_namespace = PeerRequest::SubscribeNamespace(SubscribeNamespace {
            request_id: 1,
            track_namespace_prefix: track.namespace().clone(),
            parameters: vec![],
        });
        let track_status = PeerRequest::TrackStatus(Subscribe {
            request_id: 1,
            full_track_name: track,
            parameters: vec![],
        });
        // The rows of `fixtures/client-refusals/refusals.tsv`, which
        // `@xmoq/moqt` reads as well.
        let table = include_str!("../../../fixtures/client-refusals/refusals.tsv");
        let mut read = 0;
        for line in table.lines() {
            let [name, serves, code] = line.split('\t').collect::<Vec<_>>()[..] else {
                panic!("a row of three fields: {line}");
            };
            let request = match name {
                "subscribe" => &subscribe,
                "publish" => &publish,
                "publish-namespace" => &publish_namespace,
                "fetch" => &fetch,
                "subscribe-namespace" => &subscribe_namespace,
                "track-status" => &track_status,
                other => panic!("no request {other}"),
            };
            let code = u64::from_str_radix(code.trim_start_matches("0x"), 16).unwrap();
            let serves = match serves {
                "yes" => true,
                "no" => false,
                other => panic!("no serves {other}"),
            };
            assert_eq!(refusal_code(serves, request), code, "{line}");
            read += 1;
        }
        assert_eq!(read, 11);
        // Not in the table: a SUBSCRIBE in the mode that serves one is the
        // application's to answer in `@xmoq/moqt`; this client refuses one
        // for another track with DOES_NOT_EXIST.
        assert_eq!(
            refusal_code(true, &subscribe),
            request_error_code::DOES_NOT_EXIST
        );
    }

    #[test]
    fn a_subscribe_has_the_largest_object_filter_unless_the_flag_says_otherwise() {
        // (the flags after the ones every run has; the filter of the
        //  SUBSCRIBE)
        let cases: [(&[&str], _); 4] = [
            (&[], Some(SubscriptionFilter::LargestObject)),
            (&["--subscription-filter", "none"], None),
            (
                &["--subscription-filter", "next-group-start"],
                Some(SubscriptionFilter::NextGroupStart),
            ),
            (
                &["--subscription-filter", "absolute-start:3,2"],
                Some(SubscriptionFilter::AbsoluteStart { start: at(3, 2) }),
            ),
        ];
        for (more, filter) in cases {
            let cert = "00".repeat(32);
            let always = [
                "--mode",
                "subscribe",
                "--url",
                "https://127.0.0.1:1",
                "--cert-sha256",
                cert.as_str(),
                "--namespace",
                "xmoq",
                "--track",
                "a",
            ];
            let flags = always.iter().chain(more).map(ToString::to_string);
            let args = parse_args(flags).unwrap();
            assert_eq!(args.subscription_filter, filter, "{more:?}");
        }
    }

    #[test]
    fn the_subscription_filter_of_the_flag_is_read() {
        let read = |text| parse_subscription_filter(text).map_err(|error| error.to_string());
        assert_eq!(read("none"), Ok(None));
        assert_eq!(
            read("largest-object"),
            Ok(Some(SubscriptionFilter::LargestObject))
        );
        assert_eq!(
            read("next-group-start"),
            Ok(Some(SubscriptionFilter::NextGroupStart))
        );
        assert_eq!(
            read("absolute-start:3,2"),
            Ok(Some(SubscriptionFilter::AbsoluteStart { start: at(3, 2) }))
        );
        // The End Group, which the filter carries as a delta to the Group
        // of the Start Location.
        let range = |start, end_group_delta| {
            Ok(Some(SubscriptionFilter::AbsoluteRange {
                start,
                end_group_delta,
            }))
        };
        assert_eq!(read("absolute-range:3,2,3"), range(at(3, 2), 0));
        assert_eq!(read("absolute-range:3,2,9"), range(at(3, 2), 6));
        assert_eq!(read("absolute-range:0,0,0"), range(at(0, 0), 0));
        for wrong in [
            "",
            "largest",
            "absolute-start",
            "absolute-start:3",
            "absolute-range:3,2",
            "absolute-range:3,2,x",
            // An End Group before the Start Location.
            "absolute-range:3,2,2",
        ] {
            assert!(read(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn largest_object_is_there_once_an_object_was_sent() {
        let mut largest = Largest::default();
        assert_eq!(largest.parameter(), None);
        // Object 0 of each Group, and the last one sent counts.
        for (group_id, location) in [(0, at(0, 0)), (7, at(7, 0)), (8, at(8, 0))] {
            largest.sent(group_id);
            assert_eq!(
                largest.parameter(),
                Some(Parameter::LargestObject(location)),
                "{group_id}"
            );
        }
    }

    #[test]
    fn the_lines_of_a_driver_are_told_apart() {
        let cases = [
            ("send", PublisherLine::Send),
            ("", PublisherLine::Send),
            ("republish", PublisherLine::Republish { same_alias: false }),
            (
                "republish-same-alias",
                PublisherLine::Republish { same_alias: true },
            ),
            ("publish-done", PublisherLine::PublishDone),
            ("end-of-track", PublisherLine::EndOfTrack),
            (
                "subgroup-again-longer",
                PublisherLine::SubgroupAgain { longer: true },
            ),
            (
                "subgroup-again-shorter",
                PublisherLine::SubgroupAgain { longer: false },
            ),
            ("subgroup-again-second", PublisherLine::SubgroupAgainSecond),
            (
                "publish-anew",
                PublisherLine::PublishAnew { same_alias: false },
            ),
            (
                "publish-anew-same-alias",
                PublisherLine::PublishAnew { same_alias: true },
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(PublisherLine::read(line), expected, "{line:?}");
        }
    }

    #[test]
    fn an_object_property_is_read_as_the_object_line_prints_it() {
        let cases = [
            // An even type has an integer, an odd type bytes (§1.4.3).
            ("16=1760140800123456", "16:1760140800123456"),
            ("13=0142e01f", "13:0142e01f"),
            ("13=", "13:"),
            ("0=0", "0:0"),
        ];
        for (flag, printed) in cases {
            let pair = parse_object_property(flag).unwrap();
            assert_eq!(report::properties(&[pair]), printed, "{flag}");
        }
        let both = [
            parse_object_property("13=01").unwrap(),
            parse_object_property("16=7").unwrap(),
        ];
        assert_eq!(report::properties(&both), "13:01,16:7");
        assert_eq!(report::properties(&[]), "none");
    }

    #[test]
    fn a_publisher_priority_is_a_byte_or_left_to_the_default() {
        let cases = [
            ("0", Some(0)),
            ("128", Some(128)),
            ("255", Some(255)),
            ("default", None),
        ];
        for (flag, priority) in cases {
            assert_eq!(parse_publisher_priority(flag).unwrap(), priority, "{flag}");
        }
        // Past a byte, below it, no number, and another word.
        for flag in ["256", "-1", "1.5", "", "Default", "none"] {
            assert!(parse_publisher_priority(flag).is_err(), "{flag}");
        }
    }

    #[test]
    fn an_object_property_that_does_not_fit_its_type_is_refused() {
        // No value at all, bytes for an even type, an integer that is no
        // hexadecimal for an odd one, a type that is no number.
        for flag in ["16", "16=0x10", "13=xyz", "video=1"] {
            assert!(parse_object_property(flag).is_err(), "{flag}");
        }
    }

    /// The flags of a FETCH with these set and no other.
    fn flags(set: impl FnOnce(&mut FetchFlags)) -> FetchFlags {
        let mut flags = FetchFlags::default();
        set(&mut flags);
        flags
    }

    fn at(group: u64, object: u64) -> Location {
        Location { group, object }
    }

    #[test]
    fn the_flags_of_a_fetch_are_read() {
        // (flag; value; the flags after it alone)
        let cases = [
            ("--fetch-start", "3,7", flags(|f| f.start = Some(at(3, 7)))),
            ("--fetch-end", "4,0", flags(|f| f.end = Some(at(4, 0)))),
            ("--fetch-count", "17", flags(|f| f.count = Some(17))),
            (
                "--fill-timeout-ms",
                "0",
                flags(|f| f.fill_timeout_ms = Some(0)),
            ),
            (
                "--group-order",
                "descending",
                flags(|f| f.group_order = Some(GroupOrder::Descending)),
            ),
            (
                "--group-order",
                "ascending",
                flags(|f| f.group_order = Some(GroupOrder::Ascending)),
            ),
            ("--join", "relative:2", flags(|f| f.join = Some((true, 2)))),
            ("--join", "absolute:0", flags(|f| f.join = Some((false, 0)))),
            (
                "--fetch-read-after-ms",
                "13000",
                flags(|f| f.read_after_ms = 13_000),
            ),
            (
                "--fetch-read-every-ms",
                "500",
                flags(|f| f.read_every_ms = 500),
            ),
            (
                "--fetch-cancel-after-objects",
                "2",
                flags(|f| f.cancel_after_objects = Some(2)),
            ),
            (
                "--fetch-cancel-after-ms",
                "1500",
                flags(|f| f.cancel_after_ms = Some(1_500)),
            ),
            ("--fetch-stay-ms", "3000", flags(|f| f.stay_ms = 3_000)),
            (
                "--fetch-answer-delay-ms",
                "6000",
                flags(|f| f.answer_delay_ms = 6_000),
            ),
            (
                "--fetch-stream-delay-ms",
                "12000",
                flags(|f| f.stream_delay_ms = 12_000),
            ),
            (
                "--fetch-write-every-ms",
                "500",
                flags(|f| f.write_every_ms = 500),
            ),
        ];
        for (flag, value, expected) in cases {
            let mut read = FetchFlags::default();
            assert!(read.read(flag, value).unwrap(), "{flag}");
            assert_eq!(read, expected, "{flag} {value}");
        }
        // A flag that is not about FETCH is left to the caller.
        assert!(!FetchFlags::default().read("--objects", "1").unwrap());
        // No comma, no number, no kind, another kind, another order.
        for (flag, value) in [
            ("--fetch-start", "3"),
            ("--fetch-end", "a,b"),
            ("--join", "2"),
            ("--join", "backwards:2"),
            ("--join", "relative:x"),
            ("--group-order", "random"),
            ("--fetch-count", "-1"),
        ] {
            assert!(
                FetchFlags::default().read(flag, value).is_err(),
                "{flag} {value}"
            );
        }
    }

    #[test]
    fn flags_that_go_together_are_taken() {
        let range = |f: &mut FetchFlags| (f.start, f.end) = (Some(at(0, 0)), Some(at(2, 0)));
        // (mode; flags)
        let cases = [
            ("fetch", flags(range)),
            ("fetch", flags(|f| (range(f), f.count = Some(32)).1)),
            (
                "fetch",
                flags(|f| (range(f), f.read_every_ms = 500, f.fill_timeout_ms = Some(0)).2),
            ),
            ("subscribe", flags(|_| ())),
            ("subscribe", flags(|f| f.join = Some((true, 1)))),
            (
                "subscribe",
                flags(|f| (f.join, f.join_at_once) = (Some((false, 0)), true)),
            ),
            (
                "subscribe",
                flags(|f| (f.join, f.cancel_after_objects) = (Some((true, 1)), Some(1))),
            ),
            ("publish", flags(|_| ())),
            ("publish-namespace", flags(|_| ())),
            (
                "fetch",
                flags(|f| (range(f), f.cancel_after_ms = Some(1_500)).1),
            ),
            (
                "fetch",
                flags(|f| {
                    range(f);
                    (f.cancel_after_ms, f.cancel_request_only, f.stay_ms) = (Some(1), true, 9);
                }),
            ),
            (
                "subscribe",
                flags(|f| (f.join, f.cancel_after_ms) = (Some((true, 1)), Some(1))),
            ),
            ("publish", flags(|f| f.answer = true)),
            ("publish-namespace", flags(|f| f.answer = true)),
            (
                "publish",
                flags(|f| (f.answer, f.answer_delay_ms) = (true, 6_000)),
            ),
            (
                "publish-namespace",
                flags(|f| (f.answer, f.stream_delay_ms, f.write_every_ms) = (true, 1, 1)),
            ),
            // The same Group, to its end or to a later Object.
            (
                "fetch",
                flags(|f| (f.start, f.end) = (Some(at(1, 3)), Some(at(1, 0)))),
            ),
            (
                "fetch",
                flags(|f| (f.start, f.end) = (Some(at(1, 3)), Some(at(1, 4)))),
            ),
        ];
        for (mode, flags) in cases {
            assert_eq!(flags.misuse(mode), None, "{mode} {flags:?}");
        }
    }

    #[test]
    fn flags_that_do_not_go_together_are_refused() {
        const RANGE: &str = "--fetch-start, --fetch-end and --fetch-count belong to --mode fetch";
        const ANY: &str =
            "the flags of a FETCH belong to --mode fetch and to --mode subscribe with --join";
        const ANSWERS: &str = "--answer-fetch belongs to the modes that publish";
        const REQUEST_ONLY: &str = "--fetch-cancel-request-only belongs to --mode fetch with --fetch-cancel-after-ms, and not with --fetch-cancel-after-objects";
        const PACE: &str = "--fetch-answer-delay-ms, --fetch-stream-delay-ms and --fetch-write-every-ms need --answer-fetch";
        let range = |f: &mut FetchFlags| (f.start, f.end) = (Some(at(0, 0)), Some(at(2, 0)));
        // (mode; flags; why they are refused)
        let cases = [
            ("subscribe", flags(range), RANGE),
            ("publish", flags(|f| f.count = Some(2)), RANGE),
            (
                "fetch",
                flags(|f| (range(f), f.join = Some((true, 1))).1),
                "--join belongs to --mode subscribe",
            ),
            (
                "publish",
                flags(|f| f.join = Some((true, 1))),
                "--join belongs to --mode subscribe",
            ),
            (
                "subscribe",
                flags(|f| f.join_at_once = true),
                "--join-at-once needs --join",
            ),
            ("subscribe", flags(|f| f.read_after_ms = 1), ANY),
            ("subscribe", flags(|f| f.fill_timeout_ms = Some(0)), ANY),
            (
                "publish-namespace",
                flags(|f| f.group_order = Some(GroupOrder::Descending)),
                ANY,
            ),
            ("publish", flags(|f| f.cancel_after_objects = Some(1)), ANY),
            (
                "fetch",
                flags(|f| (range(f), f.count = Some(0)).1),
                "--fetch-count is 1 to 32",
            ),
            (
                "fetch",
                flags(|f| (range(f), f.count = Some(33)).1),
                "--fetch-count is 1 to 32",
            ),
            (
                "fetch",
                flags(|f| (range(f), f.cancel_after_objects = Some(0)).1),
                "--fetch-cancel-after-objects 0 is never reached",
            ),
            // Without something that cancels, and in a mode that does not
            // stay.
            (
                "fetch",
                flags(|f| (range(f), f.cancel_request_only = true).1),
                REQUEST_ONLY,
            ),
            // With a number of Objects to cancel after, alone and beside a
            // time.
            (
                "fetch",
                flags(|f| {
                    range(f);
                    (f.cancel_after_objects, f.cancel_request_only) = (Some(2), true);
                }),
                REQUEST_ONLY,
            ),
            (
                "fetch",
                flags(|f| {
                    range(f);
                    (f.cancel_after_objects, f.cancel_after_ms) = (Some(2), Some(1));
                    f.cancel_request_only = true;
                }),
                REQUEST_ONLY,
            ),
            (
                "subscribe",
                flags(|f| {
                    f.join = Some((true, 1));
                    (f.cancel_after_ms, f.cancel_request_only) = (Some(1), true);
                }),
                REQUEST_ONLY,
            ),
            (
                "subscribe",
                flags(|f| (f.join, f.stay_ms) = (Some((true, 1)), 1)),
                "--fetch-stay-ms belongs to --mode fetch",
            ),
            ("publish", flags(|f| f.cancel_after_ms = Some(1)), ANY),
            ("subscribe", flags(|f| f.answer = true), ANSWERS),
            ("fetch", flags(|f| (range(f), f.answer = true).1), ANSWERS),
            ("publish", flags(|f| f.answer_delay_ms = 1), PACE),
            ("publish", flags(|f| f.stream_delay_ms = 1), PACE),
            ("publish-namespace", flags(|f| f.write_every_ms = 1), PACE),
        ];
        for (mode, flags, why) in cases {
            assert_eq!(flags.misuse(mode), Some(why), "{mode} {flags:?}");
        }
    }

    #[test]
    fn a_fetch_is_given_up_as_the_flags_say() {
        let ms = Duration::from_millis;
        let reading = |flags: FetchFlags| {
            let args = flags.args();
            let reading = args.reading;
            (
                reading.cancel_after_objects,
                reading.cancel_after,
                reading.cancel_request_only,
                args.stay,
            )
        };
        // Without the flags: never, both streams, and the client leaves at
        // once.
        assert_eq!(reading(flags(|_| ())), (None, None, false, ms(0)));
        let all = flags(|f| {
            (f.cancel_after_objects, f.cancel_after_ms) = (Some(2), Some(1_500));
            (f.cancel_request_only, f.stay_ms) = (true, 3_000);
        });
        assert_eq!(reading(all), (Some(2), Some(ms(1_500)), true, ms(3_000)));
        // The flag without a value is read where the others are.
        let cert = "00".repeat(32);
        let line = [
            "--mode",
            "fetch",
            "--url",
            "https://127.0.0.1:1",
            "--cert-sha256",
            cert.as_str(),
            "--namespace",
            "xmoq",
            "--track",
            "a",
            "--fetch-start",
            "0,0",
            "--fetch-end",
            "1,0",
            "--fetch-cancel-after-ms",
            "1500",
            "--fetch-cancel-request-only",
        ];
        let args = parse_args(line.iter().map(ToString::to_string)).unwrap();
        assert!(args.fetch.reading.cancel_request_only);
        assert_eq!(args.fetch.reading.cancel_after, Some(ms(1_500)));
    }

    #[test]
    fn a_fetch_is_answered_with_the_flag_and_at_the_pace_of_the_flags() {
        let ms = Duration::from_millis;
        // Without `--answer-fetch`: no FETCH is answered.
        assert_eq!(flags(|_| ()).args().answering, None);
        // With it, at once, or as slowly as the other three say.
        assert_eq!(
            flags(|f| f.answer = true).args().answering,
            Some(Pace::default())
        );
        let slow = flags(|f| {
            f.answer = true;
            (f.answer_delay_ms, f.stream_delay_ms, f.write_every_ms) = (6_000, 12_000, 500);
        });
        assert_eq!(
            slow.args().answering,
            Some(Pace {
                answer_after: ms(6_000),
                stream_after: ms(12_000),
                write_every: ms(500),
            })
        );

        let fetch = PeerRequest::Fetch(Fetch {
            request_id: 1,
            range: FetchRange::RelativeJoining {
                joining_request_id: 0,
                joining_start: 0,
            },
            parameters: vec![],
        });
        let subscribe = PeerRequest::Subscribe(Subscribe {
            request_id: 1,
            full_track_name: FullTrackName::new(
                TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap(),
                b"a".to_vec(),
            )
            .unwrap(),
            parameters: vec![],
        });
        // (whether the client has the flag; the request; whether it is one
        //  the client plans an answer for)
        let cases = [
            (None, &fetch, false),
            (Some(Pace::default()), &fetch, true),
            (Some(Pace::default()), &subscribe, false),
            (None, &subscribe, false),
        ];
        for (answering, request, answered) in cases {
            assert_eq!(
                fetch_to_answer(answering, request).is_some(),
                answered,
                "{answering:?} {request:?}"
            );
        }
        // And what is not answered gets NOT_SUPPORTED (0x3), in either mode.
        assert_eq!(
            (refusal_code(false, &fetch), refusal_code(true, &fetch)),
            (0x3, 0x3)
        );
    }

    #[test]
    fn closes_with_the_code_of_the_protocol_error() {
        let violation = || xmoq_moqt::Error::ProtocolViolation("test");
        let cases: Vec<(Result<()>, u32)> = vec![
            (Ok(()), 0x0),
            (Err(anyhow!("stdin closed")), 0x0),
            (Err(violation().into()), 0x3),
            (
                Err(anyhow::Error::from(violation()).context("reading")),
                0x3,
            ),
            (Err(xmoq_moqt::Error::DuplicateTrackAlias(7).into()), 0x5),
            (
                Err(xmoq_moqt::Error::UnsupportedMessageType(0x10).into()),
                0x1,
            ),
            (
                Err(xmoq_moqt::Error::BufferLimitExceeded { limit: 8 }.into()),
                0x1,
            ),
        ];
        for (result, code) in cases {
            assert_eq!(termination_code(&result), code, "{result:?}");
        }
    }
}
