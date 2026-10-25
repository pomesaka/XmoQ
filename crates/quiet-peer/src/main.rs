//! A peer for the interop runs, and for nothing else: it takes WebTransport
//! sessions as a relay does, and stays quiet where a relay answers.
//!
//! WHY a program of its own, and no flag of the relay: a way not to answer
//! has no place in the binary that is run for real.
//!
//! `--quiet setup`: it sends no SETUP. `--quiet requests`: it sends its
//! SETUP, reads every request the client opens, and answers none.
//! `--quiet publish`: as `requests`, but a `PUBLISH_NAMESPACE` is answered
//! with `REQUEST_OK`. `--quiet streams`: it sends its SETUP and allows the
//! client no bidirectional stream, so that no request can be sent.
//! `--quiet subgroups`: it answers a `PUBLISH_NAMESPACE` and a PUBLISH with
//! `REQUEST_OK` and allows the client no unidirectional stream beyond its
//! control stream, so that no subgroup stream can be opened.
//! `--quiet goaway`: on its first session it refuses every request with
//! `GOING_AWAY`, and after the first refusal sends a GOAWAY with the New
//! Session URI `https://127.0.0.1:<port>/moved` (`GOAWAY_SENT session=0
//! uri=..`). On later sessions it answers a SUBSCRIBE with `SUBSCRIBE_OK`
//! and sends nothing on it, and a `PUBLISH_NAMESPACE` with `REQUEST_OK`.
//! `--quiet goaway-request`: the same answers on every session, and on the
//! first one a GOAWAY on the stream of the first SUBSCRIBE, after its
//! answer, naming the same URI (`REQUEST_GOAWAY_SENT session=0 stream=0
//! uri=..`).
//! `--quiet request-id-parity`: when the client's SETUP has come, it sends the
//! client a SUBSCRIBE with the Request ID 0, a client's (§10.1), and answers
//! the client's SUBSCRIBEs with `SUBSCRIBE_OK`.
//! `--quiet request-id-again`: a SUBSCRIBE with the Request ID 1, and after
//! its answer a `REQUEST_UPDATE` on its stream with the same Request ID; the
//! client's SUBSCRIBEs as in `request-id-parity`.
//! `--quiet datagram`: when the client's SETUP has come, a datagram of the
//! type 0x30, which is no `OBJECT_DATAGRAM` (§11.3.1), and no answer to a
//! request. `--quiet datagram-before-setup`: the same datagram, and no SETUP,
//! so that the client has it while it waits for the SETUP.
//! `--quiet silent-subgroup`: it answers a SUBSCRIBE with `SUBSCRIBE_OK` and
//! a `PUBLISH_NAMESPACE` with `REQUEST_OK`, and after the `SUBSCRIBE_OK`
//! opens a subgroup stream of the track, writes its Subgroup Header and
//! nothing more, and keeps it open until the client stops it
//! (`SUBGROUP_HEADER_SENT ..`, then `STOP_SENDING_RECEIVED session=N
//! code=0x.. after_ms=M`). The session stays.
//!
//! `--connect <url> --cert-sha256 <hex> --stay-ms <n>`: the other way
//! round. It connects to a relay as a client, sends no SETUP and opens no
//! stream (with `--send-setup yes` it sends its SETUP first, and nothing
//! after it), and says how the relay ended the session
//! (`SESSION_ENDED ...`), or after that long that it did not
//! (`STAYED ms=N`). With `--send-datagram-hex <hex>` it sends that datagram
//! after its SETUP (`DATAGRAM_SENT bytes=..`).
//!
//! With `--announce-per-second <n>` as well it is not quiet: it sends its
//! SETUP, and then, that many times a second for as long as it stays, a
//! `PUBLISH_NAMESPACE` for the namespace `(xmoq)`, which it cancels as
//! soon as it is answered. A SUBSCRIBE the relay sends it meanwhile is
//! refused. At the end it says what it counted
//! (`ANNOUNCED sent=.. ok=.. refused=.. failed=.. asked=.. retry_interval=..
//! ms=..`, the Retry Interval being that of the refusal that came last): the
//! measure of what a session that announces again and again costs a relay.
//!
//! With `--ask-per-second <n> --ask subscribe|fetch-nothing|fetch-old` (or
//! `--ask-every-ms <n>` for fewer than one a second) it
//! is a subscriber that asks for something else each time: that many
//! requests a second, each for a track of `(xmoq)` whose name was not used
//! before (`subscribe`: a SUBSCRIBE for `s0`, `s1`, ..., with a
//! `RENDEZVOUS_TIMEOUT` where `--rendezvous-ms` gives one; `fetch-nothing`:
//! a Standalone FETCH for Group 0 of such a track; `fetch-old`: one for
//! Group 0 of the track `a`; `subscribe-own`: a SUBSCRIBE for `own0`,
//! `own1`, ...; `fetch-live`: a FETCH for Group 0 of the track `live`, the
//! same each time). With `--cancel yes` each is reset when it is answered,
//! or 50 ms after it was sent; with `--cancel answered` when it is
//! answered, however long that takes. With `--say-each yes` it says each
//! answer as it comes (`ANSWER n=.. after_ms=.. ok|0x..`). At the end it
//! says how the requests were answered (`ASKED sent=.. ok=.. refused=..
//! [0x..=..] failed=.. unanswered=.. ms=..`).
//!
//! With `--announce-and-refuse yes` it is a publisher that has nothing: it
//! announces `(xmoq)` once, refuses what the relay asks of it, and says at
//! the end how many requests it was sent (`REFUSED asked=..`). With
//! `--announce-and-stay-quiet yes` it publishes the track `live` as well
//! (PUBLISH, no Object), and answers nothing the relay asks of it
//! (`QUIET asked=..`); with `--quiet-track <namespace>/<name>` as well, that
//! track in place of `live` (`PUBLISHED_QUIET track=..`), so that the relay
//! has it as the first source of a track another publisher serves.
//!
//! With `--publish-group <objects>x<bytes>` it publishes the track `big` of
//! `(xmoq)` and sends Group 0 with that many Objects of that many bytes on
//! one subgroup stream (`GROUP_SENT group=0 bytes=..`), and with
//! `--next-group-after-ms <n>` Group 1, one small Object, that much later.
//!
//! With `--fetch-cached <n> --read-bytes-a-second <n>` it sends that many
//! Standalone FETCH requests for Group 0 of `big` (or the Group
//! `--fetch-group` names) and reads each fetch stream at
//! that rate (0: not at all). It says every 5 seconds how much it has read
//! (`READ bytes=.. open=..`), and how each stream ended
//! (`FETCH_STREAM_ENDED after_ms=.. bytes=.. how=..`).
//!
//! With `--quiet half-object --then finish|reset --hold-ms <n>` it
//! publishes the track `half` of `(xmoq, phase0)`, the namespace of the
//! interop runs, sends Object 0 of Group 0 whole
//! on a stream of its own, then writes the Subgroup Header of Group 1, the
//! fields of its Object 0 and half of that Object's payload of 512 KiB
//! (`HALF_OBJECT_SENT group=1 object=0`). After `--hold-ms`, or with
//! `--hold-until-line` when the driver writes the line `rest` on stdin, it
//! writes the rest, Object 1 of 16 bytes after it, and finishes the stream (`then
//! finish`); or it resets the stream with `CANCELLED` and sends Object 0 of
//! Group 2, 16 bytes, on a stream of its own (`then reset`). It says which
//! (`HALF_OBJECT_ENDED how=..`). The Object after it is what a subscriber
//! that joined while the Object was on its way gets on its subscription.
//! What the relay asks of it meanwhile, a FETCH for the Object among it,
//! is refused with `DOES_NOT_EXIST` (`REFUSED asked=..` at the end): the
//! relay that holds a viewer's FETCH for an Object it is receiving can be
//! watched to hold it, to answer it from its cache when the Object is
//! whole, and to ask the publisher when it is not.
//!
//! With `--hold-tracks <n>` it is that many viewers that came early: it
//! opens a session for every 16 of them, and sends a SUBSCRIBE for each of
//! the tracks `h0`, `h1`, ... of `(xmoq)` with a `RENDEZVOUS_TIMEOUT` of 30
//! seconds, which a relay holds. It says `HOLDING tracks=N` when all are
//! sent, and stays.
//!
//! It prints what it saw, one line each, for a driver to wait for:
//!
//! - `LISTENING port=P`
//! - `SESSION session=N path=P`: P is the path the client asked for
//! - `SETUP_RECEIVED session=N`
//! - `DATAGRAM_SENT session=N type=0x30`
//! - `REQUEST session=N stream=M <the message>`: a message on a request
//!   stream, `PUBLISH_DONE` among them, with its fields.
//! - `REQUEST_CLOSED session=N stream=M how=FIN` or `how=reset, code 0x..`:
//!   how the client ended its direction of that stream.
//! - `SESSION_ENDED session=N ClosedByPeer { code: .., reason: .. }`: how
//!   the session ended.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

use anyhow::{Context, Result, bail};
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, Fetch, FetchRange, GoAway, Publish, PublishNamespace, ReasonPhrase,
    RequestError, RequestOk, RequestUpdate, Subscribe, SubscribeOk, request_error_code,
    setup_option,
};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::Parameter;
use xmoq_moqt::session::{Role, Session, Transport};
use xmoq_moqt_io::wt::{Congestion, QuicLimits, WtConnection, WtServer, accept};
use xmoq_moqt_io::{
    Connection, RecvStream, RequestReader, RequestStreamEvent, SendStream, StreamConfig,
    StreamStops, Trace, UniEvent, WriteError, read_uni_streams, send_message,
};

/// What the peer leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quiet {
    /// Its SETUP, and everything after it.
    Setup,
    /// Its SETUP; when the client's SETUP has come, a datagram of the type
    /// 0x30, as [`Quiet::Datagram`].
    DatagramBeforeSetup,
    /// Its answers to requests.
    Requests,
    /// Room for a request: it sends its SETUP and allows the client no
    /// bidirectional stream.
    Streams,
    /// Room for a subgroup stream: it answers a `PUBLISH_NAMESPACE` and a
    /// PUBLISH with `REQUEST_OK`, as a subscriber that wants the track, and
    /// allows the client one unidirectional stream, which its control
    /// stream takes.
    Subgroups,
    /// Its answers to requests, but for a `PUBLISH_NAMESPACE`, which gets
    /// its `REQUEST_OK`: a client that announces before it publishes then
    /// comes to its PUBLISH.
    Publish,
    /// On the first session, every request refused with `GOING_AWAY`, and
    /// after the first refusal a GOAWAY that names the path `/moved` of this
    /// peer (§10.4). The next sessions get no
    /// GOAWAY: a SUBSCRIBE is answered with `SUBSCRIBE_OK`, under the number
    /// of its stream as the Track Alias, with nothing sent on it, and a
    /// `PUBLISH_NAMESPACE` with `REQUEST_OK`; other requests are not
    /// answered.
    GoAway,
    /// On the first session, a GOAWAY on the stream of the first SUBSCRIBE,
    /// after its `SUBSCRIBE_OK`, naming the path `/moved` of this peer
    /// (§10.4: "A GOAWAY MAY also be sent on a request stream to initiate
    /// migration of that individual request"). Otherwise as `GoAway` on its
    /// later sessions: a SUBSCRIBE is answered with `SUBSCRIBE_OK`, a
    /// `PUBLISH_NAMESPACE` with `REQUEST_OK`.
    RequestGoAway,
    /// When the client's SETUP has come, a SUBSCRIBE to the client with the
    /// Request ID 0, which is of a client's parity (§10.1). A SUBSCRIBE of the
    /// client is answered with `SUBSCRIBE_OK`, as in `RequestGoAway`, so that a
    /// viewer comes to play; other requests are not.
    PeerRequestIdOfTheWrongParity,
    /// When the client's SETUP has come, a SUBSCRIBE to the client with the
    /// Request ID 1, and after its answer a `REQUEST_UPDATE` on its stream
    /// with the same Request ID (§10.1: "Each ... `REQUEST_UPDATE` ...
    /// consumes a Request ID").
    PeerRequestIdAgain,
    /// When the client's SETUP has come, a datagram of the type 0x30, which
    /// is no `OBJECT_DATAGRAM` (§11.3.1), and no answer to a request: the
    /// client is to close the session while it waits for one.
    Datagram,
    /// A SUBSCRIBE is answered with `SUBSCRIBE_OK`, under the number of its
    /// stream as the Track Alias, and a `PUBLISH_NAMESPACE` with
    /// `REQUEST_OK`; after the `SUBSCRIBE_OK`, a subgroup stream of that
    /// alias with its Subgroup Header and nothing after it, kept open until
    /// the client stops it.
    SilentSubgroup,
}

/// What the program is asked to be.
enum Args {
    /// A server that takes sessions.
    Serve {
        port: u16,
        cert_file: String,
        key_file: String,
        quiet: Quiet,
    },
    /// A client that sends no SETUP.
    Connect {
        url: String,
        cert_sha256: [u8; 32],
        stay: Duration,
        /// `--send-setup yes`: it sends its SETUP, and nothing after it.
        setup: bool,
        /// `--send-datagram-hex`: after its SETUP, it sends that datagram.
        datagram: Option<Vec<u8>>,
        /// `--announce-per-second`: it announces and withdraws that often.
        announce_per_second: Option<u32>,
        /// `--hold-tracks`: it has SUBSCRIBEs for that many tracks held.
        hold_tracks: Option<u64>,
        /// `--ask-per-second` and what belongs to it.
        ask: Option<Ask>,
        /// `--announce-and-refuse yes`, and `--announce-and-stay-quiet yes`.
        announce_and_refuse: bool,
        /// `--announce-and-stay-quiet yes`: it answers nothing, and publishes
        /// this track, its namespace's fields and its name joined with `/`
        /// (`--quiet-track`, `xmoq/live` where it is not given).
        quiet_track: Option<String>,
        /// `--publish-group` and `--next-group-after-ms`.
        publish_group: Option<(u64, usize, Option<Duration>)>,
        /// `--fetch-cached`, `--read-bytes-a-second` and `--fetch-group`.
        fetch_cached: Option<(u64, usize, u64)>,
    },
}

/// `--then`: what becomes of the Object that `--quiet half-object` sends
/// half of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    /// The rest is written, and the stream finished.
    Finish,
    /// The stream is reset.
    Reset,
}

/// What `--ask` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    Subscribe,
    SubscribeOwn,
    FetchNothing,
    FetchOld,
    FetchLive,
}

/// `--cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cancel {
    Never,
    /// When it is answered, or 50 ms after it was sent.
    Soon,
    /// When it is answered.
    Answered,
}

/// `--ask-per-second`, `--ask`, `--rendezvous-ms` and `--cancel`.
#[derive(Debug, Clone, Copy)]
struct Ask {
    /// The time from one request to the next.
    every: Duration,
    what: Asked,
    rendezvous_ms: Option<u64>,
    cancel: Cancel,
    say_each: bool,
}

/// 32 bytes from 64 hexadecimal digits.
fn sha256_of(text: &str) -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    if text.len() != 64 {
        bail!("--cert-sha256 is 64 hexadecimal digits");
    }
    for (byte, index) in bytes.iter_mut().zip((0..64).step_by(2)) {
        let digits = text.get(index..index + 2).context("--cert-sha256")?;
        *byte = u8::from_str_radix(digits, 16).context("--cert-sha256")?;
    }
    Ok(bytes)
}

/// Bytes from hexadecimal digits, two a byte.
fn hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        bail!("an odd number of hexadecimal digits");
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            let digits = text.get(index..index + 2).context("not hexadecimal")?;
            Ok(u8::from_str_radix(digits, 16)?)
        })
        .collect()
}

/// `--ask`.
fn asked_of(value: &str) -> Result<Asked> {
    Ok(match value {
        "subscribe" => Asked::Subscribe,
        "fetch-nothing" => Asked::FetchNothing,
        "fetch-old" => Asked::FetchOld,
        "subscribe-own" => Asked::SubscribeOwn,
        "fetch-live" => Asked::FetchLive,
        other => bail!("--ask does not know {other}"),
    })
}

/// `--cancel`.
fn cancel_of(value: &str) -> Cancel {
    match value {
        "yes" => Cancel::Soon,
        "answered" => Cancel::Answered,
        _ => Cancel::Never,
    }
}

/// `--publish-group <objects>x<bytes>`.
fn group_of(value: &str) -> Result<(u64, usize)> {
    let (objects, bytes) = value
        .split_once('x')
        .context("--publish-group is <objects>x<bytes>")?;
    Ok((
        objects.parse().context("--publish-group")?,
        bytes.parse().context("--publish-group")?,
    ))
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args> {
    let (mut port, mut cert_file, mut key_file, mut quiet) = (None, None, None, None);
    let (mut url, mut cert_sha256, mut stay, mut setup) = (None, None, None, false);
    let mut datagram = None;
    let (mut announce_per_second, mut hold_tracks) = (None, None);
    let (mut ask_per_second, mut asked, mut rendezvous_ms) = (None, Asked::Subscribe, None);
    let (mut cancel, mut announce_and_refuse) = (Cancel::Never, false);
    let mut quiet_track = None;
    let (mut say_each, mut stay_quiet) = (false, false);
    let mut ask_every = None;
    let (mut group, mut next_group_after) = (None, None);
    let (mut fetches, mut read_a_second, mut fetch_group) = (None, 0, 0);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .with_context(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--port" => port = Some(value.parse().context("--port")?),
            "--cert-file" => cert_file = Some(value),
            "--key-file" => key_file = Some(value),
            "--connect" => url = Some(value),
            "--cert-sha256" => cert_sha256 = Some(sha256_of(&value)?),
            "--send-setup" => setup = value == "yes",
            "--send-datagram-hex" => {
                datagram = Some(hex(&value).context("--send-datagram-hex is hex")?);
            }
            "--ask-per-second" => {
                ask_per_second = Some(value.parse().context("--ask-per-second")?);
            }
            // One every so many milliseconds, where that is less than one
            // a second.
            "--ask-every-ms" => {
                let ms: u64 = value.parse().context("--ask-every-ms")?;
                if ms == 0 {
                    bail!("--ask-every-ms must be more than 0");
                }
                ask_every = Some(Duration::from_millis(ms));
                ask_per_second.get_or_insert(1);
            }
            "--ask" => asked = asked_of(&value)?,
            "--rendezvous-ms" => rendezvous_ms = Some(value.parse().context("--rendezvous-ms")?),
            "--cancel" => cancel = cancel_of(&value),
            "--say-each" => say_each = value == "yes",
            "--announce-and-stay-quiet" => {
                (announce_and_refuse, stay_quiet) = (true, value == "yes");
            }
            "--announce-and-refuse" => announce_and_refuse = value == "yes",
            "--quiet-track" => quiet_track = Some(value),
            "--publish-group" => group = Some(group_of(&value)?),
            "--next-group-after-ms" => {
                next_group_after = Some(Duration::from_millis(
                    value.parse().context("--next-group-after-ms")?,
                ));
            }
            "--fetch-cached" => fetches = Some(value.parse().context("--fetch-cached")?),
            "--fetch-group" => fetch_group = value.parse().context("--fetch-group")?,
            "--read-bytes-a-second" => {
                read_a_second = value.parse().context("--read-bytes-a-second")?;
            }
            "--hold-tracks" => hold_tracks = Some(value.parse().context("--hold-tracks")?),
            "--announce-per-second" => {
                announce_per_second = Some(value.parse().context("--announce-per-second")?);
            }
            "--stay-ms" => {
                stay = Some(Duration::from_millis(value.parse().context("--stay-ms")?));
            }
            // Read by `half_object_of`.
            "--then" | "--hold-ms" | "--hold-until-line" => {}
            "--quiet" if value == "half-object" => {}
            "--quiet" => quiet = Some(quiet_of(&value)?),
            other => bail!("unknown flag {other}"),
        }
    }
    if let Some(url) = url {
        return Ok(Args::Connect {
            url,
            cert_sha256: cert_sha256.context("--cert-sha256 is required")?,
            stay: stay.context("--stay-ms is required")?,
            setup,
            datagram,
            announce_per_second,
            hold_tracks,
            ask: ask_per_second.map(|per_second: u32| Ask {
                every: ask_every.unwrap_or(Duration::from_secs(1) / per_second.max(1)),
                what: asked,
                rendezvous_ms,
                cancel,
                say_each,
            }),
            announce_and_refuse,
            quiet_track: stay_quiet.then(|| quiet_track.unwrap_or_else(|| "xmoq/live".to_string())),
            publish_group: group.map(|(objects, bytes)| (objects, bytes, next_group_after)),
            fetch_cached: fetches.map(|fetches| (fetches, read_a_second, fetch_group)),
        });
    }
    Ok(Args::Serve {
        port: port.context("--port is required")?,
        cert_file: cert_file.context("--cert-file is required")?,
        key_file: key_file.context("--key-file is required")?,
        quiet: quiet.context("--quiet is required")?,
    })
}

/// How long `--quiet half-object` holds the rest of the Object back.
#[derive(Clone, Copy)]
enum Hold {
    /// `--hold-ms`.
    For(Duration),
    /// `--hold-until-line rest`: until the driver writes the line `rest`.
    ///
    /// WHY a line: the viewer has to reach the relay while the Object is
    /// half there, and a viewer that starts after the first half was sent
    /// does so within a time only on a machine that is fast enough.
    UntilLine,
}

impl Hold {
    /// Resolves when the rest of the Object is due.
    async fn over(self) {
        match self {
            Self::For(wait) => tokio::time::sleep(wait).await,
            Self::UntilLine => {
                let mut stdin = BufReader::new(tokio::io::stdin()).lines();
                match stdin.next_line().await {
                    Ok(Some(line)) if line.trim() == "rest" => {}
                    // The driver is gone, or wrote another line: the rest
                    // is never due, and the run ends with `stay`.
                    _ => std::future::pending::<()>().await,
                }
            }
        }
    }
}

/// `--quiet half-object` with its `--then` and `--hold-ms` or
/// `--hold-until-line`, where the arguments have it: a client's mode, where
/// the other values of `--quiet` are a server's.
fn half_object_of(args: &[String]) -> Result<Option<(Then, Hold)>> {
    let (mut half_object, mut then, mut hold) = (false, Then::Finish, Hold::For(Duration::ZERO));
    for pair in args.chunks(2) {
        let [flag, value] = pair else { break };
        match (flag.as_str(), value.as_str()) {
            ("--quiet", "half-object") => half_object = true,
            ("--then", "finish") => then = Then::Finish,
            ("--then", "reset") => then = Then::Reset,
            ("--then", other) => bail!("--then is finish or reset, not {other}"),
            ("--hold-ms", ms) => {
                hold = Hold::For(Duration::from_millis(ms.parse().context("--hold-ms")?));
            }
            ("--hold-until-line", "rest") => hold = Hold::UntilLine,
            ("--hold-until-line", other) => bail!("--hold-until-line is rest, not {other}"),
            _ => {}
        }
    }
    Ok(half_object.then_some((then, hold)))
}

/// The value of `--quiet`.
fn quiet_of(value: &str) -> Result<Quiet> {
    Ok(match value {
        "setup" => Quiet::Setup,
        "requests" => Quiet::Requests,
        "publish" => Quiet::Publish,
        "streams" => Quiet::Streams,
        "subgroups" => Quiet::Subgroups,
        "goaway" => Quiet::GoAway,
        "goaway-request" => Quiet::RequestGoAway,
        "request-id-parity" => Quiet::PeerRequestIdOfTheWrongParity,
        "request-id-again" => Quiet::PeerRequestIdAgain,
        "datagram" => Quiet::Datagram,
        "datagram-before-setup" => Quiet::DatagramBeforeSetup,
        "silent-subgroup" => Quiet::SilentSubgroup,
        other => bail!(
            "--quiet is setup, requests, publish, streams, subgroups, goaway, goaway-request, request-id-parity, request-id-again, datagram, datagram-before-setup or silent-subgroup, not {other}"
        ),
    })
}

/// What a message on a request stream is printed as.
fn described(message: &ControlMessage) -> String {
    match message {
        ControlMessage::PublishDone(done) => format!(
            "PUBLISH_DONE status_code={:#x} stream_count={} reason={:?}",
            done.status_code,
            done.stream_count,
            String::from_utf8_lossy(done.error_reason.as_bytes())
        ),
        ControlMessage::Publish(_) => "PUBLISH".to_string(),
        ControlMessage::PublishNamespace(_) => "PUBLISH_NAMESPACE".to_string(),
        ControlMessage::Subscribe(_) => "SUBSCRIBE".to_string(),
        ControlMessage::Fetch(_) => "FETCH".to_string(),
        other => format!("{other:?}"),
    }
}

/// What the peer answers in the modes that let a client come to play: a
/// SUBSCRIBE with `SUBSCRIBE_OK`, under the number of its stream as the Track
/// Alias, and a `PUBLISH_NAMESPACE` with `REQUEST_OK`; nothing else.
fn an_answer_that_plays(message: &ControlMessage, stream: u64) -> Option<ControlMessage> {
    match message {
        ControlMessage::Subscribe(_) => Some(ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: stream,
            parameters: vec![],
            track_properties: vec![],
        })),
        ControlMessage::PublishNamespace(_) => {
            Some(ControlMessage::RequestOk(RequestOk::default()))
        }
        _ => None,
    }
}

/// Reads one request stream to its end and prints what came. The sending
/// half is kept open and unused meanwhile: that is the answer that does
/// not come.
async fn read_request(
    session: u64,
    stream: u64,
    quiet: Quiet,
    halves: (
        <WtConnection as Connection>::Send,
        <WtConnection as Connection>::Recv,
    ),
    refusal: tokio::sync::mpsc::Sender<()>,
    port: u16,
    connection: Arc<WtConnection>,
) {
    let (mut unanswered, recv) = halves;
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    loop {
        match reader.next_event().await {
            Ok(RequestStreamEvent::Message(message)) => {
                println!(
                    "REQUEST session={session} stream={stream} {}",
                    described(&message)
                );
                let answered = matches!(
                    (quiet, &message),
                    (
                        Quiet::Publish | Quiet::Subgroups,
                        ControlMessage::PublishNamespace(_)
                    ) | (Quiet::Subgroups, ControlMessage::Publish(_))
                );
                if answered {
                    let ok = ControlMessage::RequestOk(RequestOk::default());
                    let _ = send_message(&mut unanswered, &ok, &Trace::default(), "request").await;
                }
                // The ones that send the client a request of their own answer
                // its SUBSCRIBE as well, so that a viewer comes to play,
                // where it reads the peer's requests.
                if matches!(
                    quiet,
                    Quiet::RequestGoAway
                        | Quiet::PeerRequestIdOfTheWrongParity
                        | Quiet::PeerRequestIdAgain
                ) {
                    let answer = an_answer_that_plays(&message, stream);
                    let subscribe = matches!(message, ControlMessage::Subscribe(_));
                    if let Some(answer) = answer {
                        let _ =
                            send_message(&mut unanswered, &answer, &Trace::default(), "request")
                                .await;
                    }
                    if quiet == Quiet::RequestGoAway && subscribe && session == 0 && stream == 0 {
                        let uri = format!("https://127.0.0.1:{port}/moved");
                        // "Request ID: Present only when sent on the control
                        // stream."
                        let go_away = ControlMessage::GoAway(GoAway {
                            new_session_uri: uri.clone().into_bytes(),
                            timeout_ms: 5000,
                            request_id: None,
                        });
                        let _ =
                            send_message(&mut unanswered, &go_away, &Trace::default(), "request")
                                .await;
                        println!("REQUEST_GOAWAY_SENT session={session} stream={stream} uri={uri}");
                    }
                }
                if quiet == Quiet::SilentSubgroup {
                    if let Some(answer) = an_answer_that_plays(&message, stream) {
                        let _ =
                            send_message(&mut unanswered, &answer, &Trace::default(), "request")
                                .await;
                    }
                    if matches!(message, ControlMessage::Subscribe(_)) {
                        say_nothing_on_a_subgroup(&connection, session, stream).await;
                    }
                }
                if quiet == Quiet::GoAway {
                    let answer = match &message {
                        // §10.4: "Requests with a Request ID equal to or
                        // greater than the indicated value, as well as any
                        // requests that arrive after the GOAWAY, MUST be
                        // rejected with REQUEST_ERROR using error code
                        // GOING_AWAY". The GOAWAY says 0: every request.
                        _ if session == 0 => Some(ControlMessage::RequestError(RequestError {
                            error_code: request_error_code::GOING_AWAY,
                            retry_interval: 0,
                            error_reason: ReasonPhrase::default(),
                            redirect: None,
                        })),
                        _ => an_answer_that_plays(&message, stream),
                    };
                    let refused = matches!(answer, Some(ControlMessage::RequestError(_)));
                    if let Some(answer) = answer {
                        let _ =
                            send_message(&mut unanswered, &answer, &Trace::default(), "request")
                                .await;
                    }
                    if refused {
                        // Full: the GOAWAY is under way already.
                        let _ = refusal.try_send(());
                    }
                }
            }
            Ok(RequestStreamEvent::Closed(how)) => {
                println!("REQUEST_CLOSED session={session} stream={stream} how={how}");
                break;
            }
            Err(error) => {
                println!("REQUEST_FAILED session={session} stream={stream} {error}");
                break;
            }
        }
    }
    // WHY it waits here: letting go of the sending half finishes it, which
    // a client reads as the end of the request, and the half is to stay as
    // it is until the session is gone.
    std::future::pending::<()>().await;
}

/// Opens a subgroup stream under `track_alias`, writes its Subgroup Header
/// and nothing after it (`SUBGROUP_HEADER_SENT session=N track_alias=A`),
/// and keeps it open until the client stops it: then it prints the code and
/// how long after the header that was (`STOP_SENDING_RECEIVED session=N
/// code=0x.. after_ms=M`, `code=none` where it carried none).
async fn say_nothing_on_a_subgroup(connection: &WtConnection, session: u64, track_alias: u64) {
    let Ok(mut send) = connection.open_uni().await else {
        return;
    };
    let header = SubgroupHeader {
        track_alias,
        group_id: 0,
        subgroup_id: SubgroupId::Zero,
        publisher_priority: Some(0),
        properties: false,
        end_of_group: false,
        first_object: true,
    };
    if send
        .write_all(&SubgroupStreamEncoder::new(&header).take_bytes())
        .await
        .is_err()
    {
        return;
    }
    let began = tokio::time::Instant::now();
    println!("SUBGROUP_HEADER_SENT session={session} track_alias={track_alias}");
    if let WriteError::Stopped(code) = send.stopped().await {
        let code = code.map_or_else(|| "none".to_string(), |code| format!("{code:#x}"));
        println!(
            "STOP_SENDING_RECEIVED session={session} code={code} after_ms={}",
            began.elapsed().as_millis()
        );
    }
}

/// Sends the client a SUBSCRIBE with `request_id` for the track `t` of
/// `(xmoq)`, prints its answer (`ANSWERED session=N ...`), and with `again`
/// sends a `REQUEST_UPDATE` with the same Request ID on its stream
/// (`UPDATE_SENT session=N request_id=..`). The stream is kept until the
/// session ends.
async fn subscribe_to_the_client(
    connection: Arc<WtConnection>,
    session: u64,
    request_id: u64,
    again: bool,
) {
    let Ok((mut send, recv)) = connection.open_bi().await else {
        return;
    };
    let Ok(namespace) = TrackNamespace::new(vec![b"xmoq".to_vec()]) else {
        return;
    };
    let Ok(full_track_name) = FullTrackName::new(namespace, b"t".to_vec()) else {
        return;
    };
    let subscribe = ControlMessage::Subscribe(Subscribe {
        request_id,
        full_track_name,
        parameters: vec![],
    });
    if send_message(&mut send, &subscribe, &Trace::default(), "request")
        .await
        .is_err()
    {
        return;
    }
    println!("SUBSCRIBE_SENT session={session} request_id={request_id}");
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    if let Ok(event) = reader.next_event().await {
        println!("ANSWERED session={session} {event:?}");
    }
    if again {
        let update = ControlMessage::RequestUpdate(RequestUpdate {
            request_id,
            parameters: vec![],
        });
        if send_message(&mut send, &update, &Trace::default(), "request")
            .await
            .is_ok()
        {
            println!("UPDATE_SENT session={session} request_id={request_id}");
        }
    }
    std::future::pending::<()>().await;
}

async fn serve(
    session: u64,
    connection: WtConnection,
    quiet: Quiet,
    port: u16,
    path: &str,
) -> Result<()> {
    let connection = Arc::new(connection);
    println!("SESSION session={session} path={path}");
    let (queue, mut events) = tokio::sync::mpsc::channel(64);
    // WHY one stream is read where no subgroup stream is to have room: a
    // stream that is not taken keeps its place among the streams the
    // client may have open, so that the few a client has beyond its control
    // stream (how many depends on how its HTTP/3 layer counts) are used up
    // by its first Subgroups and not given back.
    let reading = if quiet == Quiet::Subgroups {
        StreamConfig {
            max_uni_streams: 1,
            ..StreamConfig::default()
        }
    } else {
        StreamConfig::default()
    };
    let _readers = read_uni_streams(
        Arc::clone(&connection),
        reading,
        StreamStops::default(),
        queue,
        |event| event,
    );
    // Kept to the end: a control stream that is let go of is finished,
    // which closes the session for the client (§3.3).
    let mut control = None;
    if !matches!(quiet, Quiet::Setup | Quiet::DatagramBeforeSetup) {
        let mut moqt = Session::new(Role::Server, Transport::WebTransport);
        let setup = moqt.setup(vec![KeyValuePair::bytes(
            setup_option::MOQT_IMPLEMENTATION,
            b"xmoq-quiet-peer/0.0.0".to_vec(),
        )?])?;
        let stream = control.insert(connection.open_uni().await?);
        send_message(stream, &setup, &Trace::default(), "control").await?;
    }
    let mut requests = tokio::task::JoinSet::new();
    let mut streams = 0u64;
    // The requests this peer opens towards the client.
    let mut asked = tokio::task::JoinSet::new();
    // A request of the first session that was refused with GOING_AWAY.
    let (refusal, mut refused) = tokio::sync::mpsc::channel::<()>(1);
    let mut went_away = false;
    let end = loop {
        tokio::select! {
            end = connection.closed() => break end,
            event = events.recv() => {
                if let Some(UniEvent::Control(ControlMessage::Setup(_))) = event {
                    println!("SETUP_RECEIVED session={session}");
                    if matches!(quiet, Quiet::Datagram | Quiet::DatagramBeforeSetup) {
                        connection.send_datagram(&[0x30])?;
                        println!("DATAGRAM_SENT session={session} type=0x30");
                    }
                    let request_id = match quiet {
                        Quiet::PeerRequestIdOfTheWrongParity => Some(0),
                        Quiet::PeerRequestIdAgain => Some(1),
                        _ => None,
                    };
                    if let Some(request_id) = request_id {
                        let again = quiet == Quiet::PeerRequestIdAgain;
                        asked.spawn(subscribe_to_the_client(
                            Arc::clone(&connection),
                            session,
                            request_id,
                            again,
                        ));
                    }
                }
            }
            // WHY after the first refusal and not before it: the client then
            // reads the refusal first, as it may from a relay, since QUIC keeps
            // no order between the two streams, and has to wait for the
            // GOAWAY behind it.
            Some(()) = refused.recv(), if quiet == Quiet::GoAway && !went_away => {
                went_away = true;
                if let Some(stream) = control.as_mut() {
                    let uri = format!("https://127.0.0.1:{port}/moved");
                    // §10.4: "If no requests have been processed, this is 0
                    // (at a server)".
                    let go_away = ControlMessage::GoAway(GoAway {
                        new_session_uri: uri.clone().into_bytes(),
                        timeout_ms: 5000,
                        request_id: Some(0),
                    });
                    send_message(stream, &go_away, &Trace::default(), "control").await?;
                    println!("GOAWAY_SENT session={session} uri={uri}");
                }
            }
            halves = connection.accept_bi() => {
                let Ok(halves) = halves else { continue };
                requests.spawn(read_request(session, streams, quiet, halves, refusal.clone(), port, Arc::clone(&connection)));
                streams += 1;
            }
        }
    };
    // What the requests' tasks still print is of streams that ended with
    // the session.
    requests.abort_all();
    asked.abort_all();
    println!("SESSION_ENDED session={session} {end:?}");
    drop(control);
    Ok(())
}

/// Connects, sends nothing, and says how the session ended or that it did
/// not.
async fn stay_quiet(
    url: &str,
    cert_sha256: [u8; 32],
    stay: Duration,
    setup: bool,
    datagram: Option<Vec<u8>>,
) -> Result<()> {
    let connection = xmoq_moqt_io::wt::connect(url, cert_sha256, &StreamConfig::default()).await?;
    println!("CONNECTED");
    // Kept to the end: a control stream that is let go of is finished.
    let mut control = None;
    if setup {
        let mut moqt = Session::new(Role::Client, Transport::WebTransport);
        let message = moqt.setup(vec![KeyValuePair::bytes(
            setup_option::MOQT_IMPLEMENTATION,
            b"xmoq-quiet-peer/0.0.0".to_vec(),
        )?])?;
        let stream = control.insert(connection.open_uni().await?);
        send_message(stream, &message, &Trace::default(), "control").await?;
        println!("SETUP_SENT");
    }
    if let Some(datagram) = datagram {
        connection.send_datagram(&datagram)?;
        println!("DATAGRAM_SENT bytes={datagram:02x?}");
    }
    if let Ok(end) = tokio::time::timeout(stay, connection.closed()).await {
        println!("SESSION_ENDED {end:?}");
    } else {
        println!("STAYED ms={}", stay.as_millis());
        connection.close(0, b"");
    }
    Ok(())
}

/// What became of the announcements, counted by the tasks that made them.
#[derive(Default)]
struct Announced {
    ok: AtomicU64,
    refused: AtomicU64,
    failed: AtomicU64,
    /// SUBSCRIBEs of the relay that were refused.
    asked: AtomicU64,
    /// The Retry Interval of the refusal that came last.
    retry_interval: AtomicU64,
}

/// One `PUBLISH_NAMESPACE` for `(xmoq)`, cancelled when it is answered.
async fn announce_once(connection: &WtConnection, request_id: u64, counts: &Announced) {
    let sent = async {
        let (mut send, recv) = connection.open_bi().await.ok()?;
        let message = ControlMessage::PublishNamespace(PublishNamespace {
            request_id,
            track_namespace: TrackNamespace::new(vec![b"xmoq".to_vec()]).ok()?,
            parameters: Vec::new(),
        });
        send_message(&mut send, &message, &Trace::default(), "request")
            .await
            .ok()?;
        let mut reader = RequestReader::new(recv, &Trace::default(), "request");
        let answer = reader.next_event().await.ok()?;
        // §9.5: "When a publisher wants to stop new subscriptions for a
        // published namespace, it cancels the request". CANCELLED (0x1).
        send.reset(0x1);
        Some(answer)
    };
    let counter = match sent.await {
        Some(RequestStreamEvent::Message(ControlMessage::RequestOk(_))) => &counts.ok,
        Some(RequestStreamEvent::Message(ControlMessage::RequestError(error))) => {
            counts
                .retry_interval
                .store(error.retry_interval, Ordering::Relaxed);
            &counts.refused
        }
        _ => &counts.failed,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

/// Refuses what the relay asks of this session: `DOES_NOT_EXIST` (0x10).
async fn refuse(
    halves: (
        <WtConnection as Connection>::Send,
        <WtConnection as Connection>::Recv,
    ),
    counts: &Announced,
) {
    let (mut send, recv) = halves;
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    if reader.next_event().await.is_err() {
        return;
    }
    let refusal = ControlMessage::RequestError(RequestError {
        error_code: 0x10,
        retry_interval: 0,
        error_reason: ReasonPhrase::default(),
        redirect: None,
    });
    if send_message(&mut send, &refusal, &Trace::default(), "request")
        .await
        .is_ok()
    {
        counts.asked.fetch_add(1, Ordering::Relaxed);
    }
    send.finish();
}

/// Connects, and announces and withdraws a namespace `per_second` times a
/// second for `stay`.
async fn announce(url: &str, cert_sha256: [u8; 32], stay: Duration, per_second: u32) -> Result<()> {
    let connection =
        Arc::new(xmoq_moqt_io::wt::connect(url, cert_sha256, &StreamConfig::default()).await?);
    println!("CONNECTED");
    let mut moqt = Session::new(Role::Client, Transport::WebTransport);
    let message = moqt.setup(vec![KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        b"xmoq-quiet-peer/0.0.0".to_vec(),
    )?])?;
    // Kept to the end: a control stream that is let go of is finished.
    let mut control = connection.open_uni().await?;
    send_message(&mut control, &message, &Trace::default(), "control").await?;
    let counts = Arc::new(Announced::default());
    let began = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(1) / per_second.max(1));
    let mut tasks = tokio::task::JoinSet::new();
    let mut sent = 0u64;
    let end = loop {
        tokio::select! {
            end = connection.closed() => break Some(end),
            () = tokio::time::sleep_until(began + stay) => break None,
            _ = tick.tick() => {
                let (connection, counts) = (Arc::clone(&connection), Arc::clone(&counts));
                // A client's Request IDs are the even numbers (§10.1).
                let request_id = sent * 2;
                sent += 1;
                tasks.spawn(async move { announce_once(&connection, request_id, &counts).await });
            }
            halves = connection.accept_bi() => {
                let Ok(halves) = halves else { continue };
                let counts = Arc::clone(&counts);
                tasks.spawn(async move { refuse(halves, &counts).await });
            }
            // Done ones are taken out, so that the set does not grow.
            Some(_) = tasks.join_next() => {}
        }
    };
    println!(
        "ANNOUNCED sent={sent} ok={} refused={} failed={} asked={} retry_interval={} ms={}",
        counts.ok.load(Ordering::Relaxed),
        counts.refused.load(Ordering::Relaxed),
        counts.failed.load(Ordering::Relaxed),
        counts.asked.load(Ordering::Relaxed),
        counts.retry_interval.load(Ordering::Relaxed),
        began.elapsed().as_millis()
    );
    match end {
        Some(end) => println!("SESSION_ENDED {end:?}"),
        None => connection.close(0, b""),
    }
    Ok(())
}

/// How the requests of `--ask` were answered, counted by their tasks.
#[derive(Default)]
struct Answers {
    ok: AtomicU64,
    failed: AtomicU64,
    /// By Error Code.
    refused: std::sync::Mutex<std::collections::BTreeMap<u64, u64>>,
}

/// One request of `--ask`, with the Request ID `request_id`.
async fn ask_once(connection: &WtConnection, number: u64, ask: Ask, answers: &Answers) {
    let asked = async {
        let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).ok()?;
        let track = |name: String| FullTrackName::new(namespace.clone(), name.into_bytes()).ok();
        // A client's Request IDs are the even numbers (§10.1).
        let request_id = number * 2;
        // Group 0, whole: from (0, 0) to "the end Location, plus 1", (1, 0).
        let group_0 = |full_track_name| FetchRange::Standalone {
            full_track_name,
            start: Location {
                group: 0,
                object: 0,
            },
            end: Location {
                group: 1,
                object: 0,
            },
        };
        let message = match ask.what {
            Asked::Subscribe => ControlMessage::Subscribe(Subscribe {
                request_id,
                full_track_name: track(format!("s{number}"))?,
                parameters: ask
                    .rendezvous_ms
                    .map(Parameter::RendezvousTimeout)
                    .into_iter()
                    .collect(),
            }),
            Asked::FetchNothing => ControlMessage::Fetch(Fetch {
                request_id,
                range: group_0(track(format!("f{number}"))?),
                parameters: Vec::new(),
            }),
            Asked::FetchOld => ControlMessage::Fetch(Fetch {
                request_id,
                range: group_0(track("a".to_string())?),
                parameters: Vec::new(),
            }),
            Asked::FetchLive => ControlMessage::Fetch(Fetch {
                request_id,
                range: group_0(track("live".to_string())?),
                parameters: Vec::new(),
            }),
            Asked::SubscribeOwn => ControlMessage::Subscribe(Subscribe {
                request_id,
                full_track_name: track(format!("own{number}"))?,
                parameters: Vec::new(),
            }),
        };
        let began = tokio::time::Instant::now();
        let (mut send, recv) = connection.open_bi().await.ok()?;
        send_message(&mut send, &message, &Trace::default(), "request")
            .await
            .ok()?;
        let mut reader = RequestReader::new(recv, &Trace::default(), "request");
        let answer = if ask.cancel == Cancel::Soon {
            tokio::time::timeout(Duration::from_millis(50), reader.next_event())
                .await
                .ok()
        } else {
            Some(reader.next_event().await)
        };
        if ask.say_each {
            let said = match &answer {
                Some(Ok(RequestStreamEvent::Message(ControlMessage::RequestError(error)))) => {
                    format!("{:#x}", error.error_code)
                }
                Some(Ok(RequestStreamEvent::Message(_))) => "ok".to_string(),
                _ => "nothing".to_string(),
            };
            println!(
                "ANSWER n={number} after_ms={} {said}",
                began.elapsed().as_millis()
            );
        }
        if ask.cancel == Cancel::Never {
            // WHY forgotten: a request stream that is let go of is reset,
            // and these stay open for as long as the program runs.
            std::mem::forget(send);
        } else {
            // CANCELLED (0x1).
            send.reset(0x1);
        }
        Some(answer)
    };
    match asked.await {
        Some(Some(Ok(RequestStreamEvent::Message(
            ControlMessage::SubscribeOk(_) | ControlMessage::FetchOk(_),
        )))) => {
            answers.ok.fetch_add(1, Ordering::Relaxed);
        }
        Some(Some(Ok(RequestStreamEvent::Message(ControlMessage::RequestError(error))))) => {
            if let Ok(mut refused) = answers.refused.lock() {
                *refused.entry(error.error_code).or_default() += 1;
            }
        }
        // Cancelled before any answer.
        Some(None) => {}
        _ => {
            answers.failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Connects and sends its SETUP. The control stream is to be kept.
async fn connected(
    url: &str,
    cert_sha256: [u8; 32],
) -> Result<(Arc<WtConnection>, <WtConnection as Connection>::Send)> {
    let connection =
        Arc::new(xmoq_moqt_io::wt::connect(url, cert_sha256, &StreamConfig::default()).await?);
    println!("CONNECTED");
    let mut moqt = Session::new(Role::Client, Transport::WebTransport);
    let message = moqt.setup(vec![KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        b"xmoq-quiet-peer/0.0.0".to_vec(),
    )?])?;
    let mut control = connection.open_uni().await?;
    send_message(&mut control, &message, &Trace::default(), "control").await?;
    Ok((connection, control))
}

/// Asks for something else `ask.per_second` times a second for `stay`.
async fn ask_again_and_again(
    url: &str,
    cert_sha256: [u8; 32],
    stay: Duration,
    ask: Ask,
) -> Result<()> {
    let (connection, _control) = connected(url, cert_sha256).await?;
    let answers = Arc::new(Answers::default());
    let began = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(ask.every);
    let mut tasks = tokio::task::JoinSet::new();
    let mut sent = 0u64;
    let end = loop {
        tokio::select! {
            end = connection.closed() => break Some(end),
            () = tokio::time::sleep_until(began + stay) => break None,
            _ = tick.tick() => {
                let (connection, answers) = (Arc::clone(&connection), Arc::clone(&answers));
                let number = sent;
                sent += 1;
                tasks.spawn(async move { ask_once(&connection, number, ask, &answers).await });
            }
            // Done ones are taken out, so that the set does not grow.
            Some(_) = tasks.join_next() => {}
        }
    };
    let (ok, failed) = (
        answers.ok.load(Ordering::Relaxed),
        answers.failed.load(Ordering::Relaxed),
    );
    let refused = answers
        .refused
        .lock()
        .map(|refused| refused.clone())
        .unwrap_or_default();
    let by_code: Vec<String> = refused
        .iter()
        .map(|(code, times)| format!("{code:#x}={times}"))
        .collect();
    let answered = ok + failed + refused.values().sum::<u64>();
    println!(
        "ASKED sent={sent} ok={ok} refused={} [{}] failed={failed} unanswered={} ms={}",
        refused.values().sum::<u64>(),
        by_code.join(" "),
        sent.saturating_sub(answered),
        began.elapsed().as_millis()
    );
    match end {
        Some(end) => println!("SESSION_ENDED {end:?}"),
        None => connection.close(0, b""),
    }
    Ok(())
}

/// Takes a request of the relay, and answers nothing: the stream is kept
/// as it is.
async fn leave_unanswered(
    halves: (
        <WtConnection as Connection>::Send,
        <WtConnection as Connection>::Recv,
    ),
    counts: &Announced,
) {
    let (unanswered, recv) = halves;
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    if reader.next_event().await.is_ok() {
        counts.asked.fetch_add(1, Ordering::Relaxed);
    }
    // To its end, which is the relay giving the request up.
    while let Ok(RequestStreamEvent::Message(_)) = reader.next_event().await {}
    drop(unanswered);
}

/// Announces `(xmoq)` once and refuses what the relay asks of it, for
/// `stay`. With `quiet_track` it publishes that track as well, and answers
/// nothing the relay asks.
async fn announce_and_refuse(
    url: &str,
    cert_sha256: [u8; 32],
    stay: Duration,
    quiet_track: Option<&str>,
) -> Result<()> {
    let answers_nothing = quiet_track.is_some();
    let (connection, _control) = connected(url, cert_sha256).await?;
    let counts = Arc::new(Announced::default());
    let message = ControlMessage::PublishNamespace(PublishNamespace {
        request_id: 0,
        track_namespace: TrackNamespace::new(vec![b"xmoq".to_vec()])?,
        parameters: Vec::new(),
    });
    // Kept to the end: the announcement lasts as long as its stream.
    let (mut announcement, recv) = connection.open_bi().await?;
    send_message(&mut announcement, &message, &Trace::default(), "request").await?;
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    let answer = reader.next_event().await;
    if !matches!(
        answer,
        Ok(RequestStreamEvent::Message(ControlMessage::RequestOk(_)))
    ) {
        bail!("the announcement was not accepted: {answer:?}");
    }
    println!("ANNOUNCED_ONCE");
    // Kept to the end, as the announcement.
    let mut publication = None;
    if answers_nothing {
        let named = quiet_track.unwrap_or("xmoq/live");
        let (namespace, name) = named
            .rsplit_once('/')
            .context("--quiet-track is <namespace>/<name>")?;
        let fields = namespace
            .split('/')
            .map(|field| field.as_bytes().to_vec())
            .collect();
        let full_track_name =
            FullTrackName::new(TrackNamespace::new(fields)?, name.as_bytes().to_vec())?;
        let publish = ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name,
            track_alias: 1,
            parameters: Vec::new(),
            track_properties: Vec::new(),
        });
        let (mut send, recv) = connection.open_bi().await?;
        send_message(&mut send, &publish, &Trace::default(), "request").await?;
        let mut reader = RequestReader::new(recv, &Trace::default(), "request");
        let answer = reader.next_event().await;
        if !matches!(
            answer,
            Ok(RequestStreamEvent::Message(ControlMessage::RequestOk(_)))
        ) {
            bail!("the PUBLISH was not accepted: {answer:?}");
        }
        publication = Some((send, reader));
        // WHY two lines: drivers wait for the first since before the flag.
        if named == "xmoq/live" {
            println!("PUBLISHED_LIVE");
        } else {
            println!("PUBLISHED_QUIET track={named}");
        }
    }
    let began = tokio::time::Instant::now();
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = connection.closed() => break,
            () = tokio::time::sleep_until(began + stay) => break,
            halves = connection.accept_bi() => {
                let Ok(halves) = halves else { continue };
                let counts = Arc::clone(&counts);
                tasks.spawn(async move {
                    if answers_nothing {
                        leave_unanswered(halves, &counts).await;
                    } else {
                        refuse(halves, &counts).await;
                    }
                });
            }
            Some(_) = tasks.join_next() => {}
        }
    }
    println!(
        "{} asked={}",
        if answers_nothing { "QUIET" } else { "REFUSED" },
        counts.asked.load(Ordering::Relaxed)
    );
    drop(publication);
    connection.close(0, b"");
    Ok(())
}

/// One Group on a subgroup stream of its own: `objects` Objects of `bytes`
/// bytes, under the Track Alias 1.
async fn send_group(
    connection: &WtConnection,
    group_id: u64,
    objects: u64,
    bytes: usize,
) -> Result<()> {
    let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
        track_alias: 1,
        group_id,
        subgroup_id: SubgroupId::Zero,
        publisher_priority: Some(0),
        properties: false,
        end_of_group: false,
        first_object: true,
    });
    let mut stream = connection.open_uni().await?;
    for object_id in 0..objects {
        encoder.object(&SubgroupObject {
            object_id,
            properties: Vec::new(),
            status: ObjectStatus::Normal,
            payload: vec![0x61; bytes],
        })?;
        if let Err(error) = stream.write_all(&encoder.take_bytes()).await {
            bail!("the subgroup stream could not be written: {error:?}");
        }
    }
    stream.finish();
    println!(
        "GROUP_SENT group={group_id} bytes={}",
        objects * u64::try_from(bytes)?
    );
    Ok(())
}

/// Publishes the track `big` with one Group of `objects` Objects of `bytes`
/// bytes, and Group 1 after `next`, where that is given.
async fn publish_group(
    url: &str,
    cert_sha256: [u8; 32],
    stay: Duration,
    (objects, bytes, next): (u64, usize, Option<Duration>),
) -> Result<()> {
    let (connection, _control) = connected(url, cert_sha256).await?;
    let publish = ControlMessage::Publish(Publish {
        request_id: 0,
        full_track_name: FullTrackName::new(
            TrackNamespace::new(vec![b"xmoq".to_vec()])?,
            b"big".to_vec(),
        )?,
        track_alias: 1,
        parameters: Vec::new(),
        track_properties: Vec::new(),
    });
    // Kept to the end: the subscription lasts as long as its stream.
    let (mut publication, recv) = connection.open_bi().await?;
    send_message(&mut publication, &publish, &Trace::default(), "request").await?;
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    let answer = reader.next_event().await;
    if !matches!(
        answer,
        Ok(RequestStreamEvent::Message(ControlMessage::RequestOk(_)))
    ) {
        bail!("the PUBLISH was not accepted: {answer:?}");
    }
    let began = tokio::time::Instant::now();
    send_group(&connection, 0, objects, bytes).await?;
    if let Some(next) = next {
        tokio::time::sleep(next).await;
        send_group(&connection, 1, 1, 16).await?;
    }
    tokio::time::sleep_until(began + stay).await;
    connection.close(0, b"");
    Ok(())
}

/// The payload of the Object `--quiet half-object` sends half of: half of
/// the relay's 1 MiB for an Object, and more than a read brings at once.
const HALF_OBJECT_BYTES: usize = 512 * 1024;

/// Publishes the track `half` and sends half of an Object of it, as
/// `--quiet half-object` has it.
async fn half_object(
    url: &str,
    cert_sha256: [u8; 32],
    stay: Duration,
    (then, hold): (Then, Hold),
) -> Result<()> {
    let (connection, _control) = connected(url, cert_sha256).await?;
    let publish = ControlMessage::Publish(Publish {
        request_id: 0,
        full_track_name: FullTrackName::new(
            TrackNamespace::new(vec![b"xmoq".to_vec(), b"phase0".to_vec()])?,
            b"half".to_vec(),
        )?,
        track_alias: 1,
        parameters: Vec::new(),
        track_properties: Vec::new(),
    });
    // Kept to the end: the subscription lasts as long as its stream.
    let (mut publication, recv) = connection.open_bi().await?;
    send_message(&mut publication, &publish, &Trace::default(), "request").await?;
    let mut reader = RequestReader::new(recv, &Trace::default(), "request");
    let answer = reader.next_event().await;
    if !matches!(
        answer,
        Ok(RequestStreamEvent::Message(ControlMessage::RequestOk(_)))
    ) {
        bail!("the PUBLISH was not accepted: {answer:?}");
    }
    let began = tokio::time::Instant::now();
    send_group(&connection, 0, 1, 16).await?;
    let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
        track_alias: 1,
        group_id: 1,
        subgroup_id: SubgroupId::Zero,
        publisher_priority: Some(0),
        properties: false,
        end_of_group: false,
        first_object: true,
    });
    encoder.object(&SubgroupObject {
        object_id: 0,
        properties: Vec::new(),
        status: ObjectStatus::Normal,
        payload: vec![0x61; HALF_OBJECT_BYTES],
    })?;
    let bytes = encoder.take_bytes();
    let (first, rest) = bytes.split_at(bytes.len() - HALF_OBJECT_BYTES / 2);
    let mut stream = connection.open_uni().await?;
    if let Err(error) = stream.write_all(first).await {
        bail!("the subgroup stream could not be written: {error:?}");
    }
    println!("HALF_OBJECT_SENT group=1 object=0");
    let counts = Arc::new(Announced::default());
    let mut tasks = tokio::task::JoinSet::new();
    let mut unended = Some(stream);
    let rest_due = hold.over();
    tokio::pin!(rest_due);
    loop {
        tokio::select! {
            _ = connection.closed() => break,
            () = tokio::time::sleep_until(began + stay) => break,
            () = &mut rest_due, if unended.is_some() => {
                let Some(mut stream) = unended.take() else { continue };
                match then {
                    Then::Finish => {
                        encoder.object(&SubgroupObject {
                            object_id: 1,
                            properties: Vec::new(),
                            status: ObjectStatus::Normal,
                            payload: vec![0x62; 16],
                        })?;
                        let next = encoder.take_bytes();
                        for bytes in [rest, next.as_slice()] {
                            if let Err(error) = stream.write_all(bytes).await {
                                bail!("the subgroup stream could not be written: {error:?}");
                            }
                        }
                        stream.finish();
                        println!("HALF_OBJECT_ENDED how=finish");
                    }
                    Then::Reset => {
                        stream.reset(xmoq_moqt::stream_reset_code::CANCELLED);
                        println!("HALF_OBJECT_ENDED how=reset");
                        send_group(&connection, 2, 1, 16).await?;
                    }
                }
            }
            halves = connection.accept_bi() => {
                let Ok(halves) = halves else { continue };
                let counts = Arc::clone(&counts);
                tasks.spawn(async move { refuse(halves, &counts).await });
            }
            Some(_) = tasks.join_next() => {}
        }
    }
    println!("REFUSED asked={}", counts.asked.load(Ordering::Relaxed));
    drop(publication);
    connection.close(0, b"");
    Ok(())
}

/// Sends `fetches` Standalone FETCH requests for the Group `group` of `big` and reads every
/// stream the relay opens at `a_second` bytes a second.
async fn fetch_cached(
    url: &str,
    cert_sha256: [u8; 32],
    stay: Duration,
    (fetches, a_second, group): (u64, usize, u64),
) -> Result<()> {
    let (connection, _control) = connected(url, cert_sha256).await?;
    let began = tokio::time::Instant::now();
    // Kept to the end: a request stream that is let go of is reset.
    let mut requests = Vec::new();
    for number in 0..fetches {
        let message = ControlMessage::Fetch(Fetch {
            request_id: number * 2,
            range: FetchRange::Standalone {
                full_track_name: FullTrackName::new(
                    TrackNamespace::new(vec![b"xmoq".to_vec()])?,
                    b"big".to_vec(),
                )?,
                start: Location { group, object: 0 },
                end: Location {
                    group: group + 1,
                    object: 0,
                },
            },
            parameters: Vec::new(),
        });
        let (mut send, recv) = connection.open_bi().await?;
        send_message(&mut send, &message, &Trace::default(), "request").await?;
        let mut reader = RequestReader::new(recv, &Trace::default(), "request");
        let answer = match reader.next_event().await {
            Ok(RequestStreamEvent::Message(ControlMessage::FetchOk(_))) => "FETCH_OK".to_string(),
            Ok(RequestStreamEvent::Message(ControlMessage::RequestError(error))) => {
                format!("REQUEST_ERROR {:#x}", error.error_code)
            }
            other => format!("{other:?}"),
        };
        println!("FETCH n={number} {answer}");
        requests.push((send, reader));
    }
    let read = Arc::new(AtomicU64::new(0));
    let open = Arc::new(AtomicU64::new(0));
    let mut said = tokio::time::interval(Duration::from_secs(5));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = connection.closed() => break,
            () = tokio::time::sleep_until(began + stay) => break,
            _ = said.tick() => println!(
                "READ after_ms={} bytes={} open={}",
                began.elapsed().as_millis(),
                read.load(Ordering::Relaxed),
                open.load(Ordering::Relaxed)
            ),
            stream = connection.accept_uni() => {
                let Ok(mut stream) = stream else { continue };
                let (read, open) = (Arc::clone(&read), Arc::clone(&open));
                open.fetch_add(1, Ordering::Relaxed);
                tasks.spawn(async move {
                    // A tenth of a second's worth at a time.
                    let mut buffer = vec![0u8; (a_second / 10).max(1)];
                    let mut own = 0u64;
                    let how = loop {
                        if a_second == 0 {
                            std::future::pending::<()>().await;
                        }
                        match stream.read(&mut buffer).await {
                            Ok(Some(length)) => {
                                let length = u64::try_from(length).unwrap_or(u64::MAX);
                                own += length;
                                read.fetch_add(length, Ordering::Relaxed);
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                            Ok(None) => break "finished".to_string(),
                            Err(error) => break format!("{error:?}"),
                        }
                    };
                    open.fetch_sub(1, Ordering::Relaxed);
                    println!(
                        "FETCH_STREAM_ENDED after_ms={} bytes={own} how={how}",
                        began.elapsed().as_millis()
                    );
                });
            }
            Some(_) = tasks.join_next() => {}
        }
    }
    println!(
        "READ after_ms={} bytes={} open={}",
        began.elapsed().as_millis(),
        read.load(Ordering::Relaxed),
        open.load(Ordering::Relaxed)
    );
    connection.close(0, b"");
    drop(requests);
    Ok(())
}

/// As many SUBSCRIBEs as one session may have held by the own relay.
const HELD_PER_SESSION: u64 = 16;

/// Has SUBSCRIBEs for `tracks` tracks nobody publishes held, and stays.
async fn hold(url: &str, cert_sha256: [u8; 32], stay: Duration, tracks: u64) -> Result<()> {
    // Kept to the end: a stream that is let go of is finished or reset.
    let mut kept = Vec::new();
    let mut sessions = Vec::new();
    for first in (0..tracks).step_by(usize::try_from(HELD_PER_SESSION)?) {
        let connection =
            xmoq_moqt_io::wt::connect(url, cert_sha256, &StreamConfig::default()).await?;
        let mut moqt = Session::new(Role::Client, Transport::WebTransport);
        let setup = moqt.setup(vec![KeyValuePair::bytes(
            setup_option::MOQT_IMPLEMENTATION,
            b"xmoq-quiet-peer/0.0.0".to_vec(),
        )?])?;
        let mut control = connection.open_uni().await?;
        send_message(&mut control, &setup, &Trace::default(), "control").await?;
        for (number, track) in (first..tracks.min(first + HELD_PER_SESSION)).enumerate() {
            let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()])?;
            let message = ControlMessage::Subscribe(Subscribe {
                // A client's Request IDs are the even numbers (§10.1).
                request_id: u64::try_from(number)? * 2,
                full_track_name: FullTrackName::new(namespace, format!("h{track}").into_bytes())?,
                parameters: vec![Parameter::RendezvousTimeout(30_000)],
            });
            let (mut send, recv) = connection.open_bi().await?;
            send_message(&mut send, &message, &Trace::default(), "request").await?;
            kept.push((send, recv));
        }
        sessions.push((connection, control));
    }
    println!("HOLDING tracks={tracks} sessions={}", sessions.len());
    tokio::time::sleep(stay).await;
    for (connection, _control) in &sessions {
        connection.close(0, b"");
    }
    Ok(())
}

/// The QUIC limits of the peer for `quiet`, with the windows the relay has.
fn quic_limits(quiet: Quiet, streams: &StreamConfig) -> QuicLimits {
    let (max_uni_streams, max_bi_streams) = xmoq_moqt_io::wt::streams_read_at_once(streams);
    let max_uni_streams = if quiet == Quiet::Subgroups {
        1
    } else {
        max_uni_streams
    };
    // As the relay has them (§11.4.2): the streams of the peer leave room
    // for its requests.
    let stream_receive_window = 128 * 1024;
    QuicLimits {
        max_uni_streams,
        // The stream the session itself is on is allowed beside these.
        max_bi_streams: if quiet == Quiet::Streams {
            0
        } else {
            max_bi_streams
        },
        stream_receive_window,
        receive_window: xmoq_moqt_io::wt::receive_window_for(
            max_uni_streams,
            stream_receive_window,
        ),
        send_window: 4 * 1024 * 1024,
        // WHY a minute, where the relay has 6 seconds: the clients are to
        // give up by their own clocks, and the connection is not to go
        // first.
        idle_timeout: Duration::from_secs(60),
        congestion: Congestion::default(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let half = half_object_of(&args)?;
    let args = match parse_args(args.into_iter())? {
        Args::Connect {
            url,
            cert_sha256,
            stay,
            ..
        } if let Some(half) = half => return half_object(&url, cert_sha256, stay, half).await,
        Args::Connect {
            url,
            cert_sha256,
            stay,
            publish_group: Some(group),
            ..
        } => return publish_group(&url, cert_sha256, stay, group).await,
        Args::Connect {
            url,
            cert_sha256,
            stay,
            fetch_cached: Some(fetching),
            ..
        } => return fetch_cached(&url, cert_sha256, stay, fetching).await,
        Args::Connect {
            url,
            cert_sha256,
            stay,
            ask: Some(ask),
            ..
        } => return ask_again_and_again(&url, cert_sha256, stay, ask).await,
        Args::Connect {
            url,
            cert_sha256,
            stay,
            announce_and_refuse: true,
            quiet_track,
            ..
        } => return announce_and_refuse(&url, cert_sha256, stay, quiet_track.as_deref()).await,
        Args::Connect {
            url,
            cert_sha256,
            stay,
            hold_tracks: Some(tracks),
            ..
        } => return hold(&url, cert_sha256, stay, tracks).await,
        Args::Connect {
            url,
            cert_sha256,
            stay,
            announce_per_second: Some(per_second),
            ..
        } => return announce(&url, cert_sha256, stay, per_second).await,
        Args::Connect {
            url,
            cert_sha256,
            stay,
            setup,
            datagram,
            ..
        } => return stay_quiet(&url, cert_sha256, stay, setup, datagram).await,
        Args::Serve {
            port,
            cert_file,
            key_file,
            quiet,
        } => (port, cert_file, key_file, quiet),
    };
    let (port, cert_file, key_file, quiet) = args;
    serve_all(port, &cert_file, &key_file, quiet).await
}

/// Takes sessions on `port`, and is quiet in each as `quiet` has it.
async fn serve_all(port: u16, cert_file: &str, key_file: &str, quiet: Quiet) -> Result<()> {
    let streams = StreamConfig::default();
    let limits = quic_limits(quiet, &streams);
    let server = WtServer::bind(port, cert_file, key_file, limits)?;
    println!("LISTENING port={port}");
    let mut session = 0u64;
    loop {
        let Some(incoming) = server.incoming().await else {
            bail!("the server stopped accepting");
        };
        let number = session;
        session += 1;
        tokio::spawn(async move {
            match accept(incoming).await {
                Ok((connection, asked)) => {
                    if let Err(error) = serve(number, connection, quiet, port, &asked.path).await {
                        println!("SESSION_FAILED session={number} {error:#}");
                    }
                }
                Err(error) => println!("HANDSHAKE_FAILED {error}"),
            }
        });
    }
}
