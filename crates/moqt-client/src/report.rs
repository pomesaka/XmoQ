//! Every line this client prints to stdout.
//!
//! The format of these lines is a contract with the scripts that drive the
//! client in interoperability runs: they wait for `SUBSCRIBED`, `PUBLISHED` and
//! `SENT`, and read the payload from `OBJECT`. The other lines are for people
//! reading a log. Keeping all of them here keeps that contract in one place.

use xmoq_moqt::Error as MoqtError;
use xmoq_moqt::data::{SubgroupHeader, SubgroupObject};
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::message::{PublishDone, Setup};

use xmoq_moqt_io::{Direction, RequestClose};

use crate::hex;

pub fn connected(url: &str) {
    println!("CONNECTED url={url}");
}

pub fn setup_ok(peer_setup: &Setup) {
    println!("SETUP_OK server_setup={peer_setup:?}");
}

pub fn subscribed(track_alias: u64) {
    println!("SUBSCRIBED track_alias={track_alias}");
}

/// WHY `payload_hex` stays the last field: the drivers read it up to the end
/// of the line.
pub fn object(header: &SubgroupHeader, object: &SubgroupObject) {
    println!(
        "OBJECT group={} object={} status={:?} properties={} payload_len={} payload_hex={}",
        header.group_id,
        object.object_id,
        object.status,
        properties(&object.properties),
        object.payload.len(),
        hex::encode(&object.payload),
    );
}

/// Object Properties as `<type>:<value>` joined by commas, in the order they
/// were on the wire: the type in decimal, the value of an even type in
/// decimal and that of an odd type in hexadecimal. `none` for an Object
/// without any, which keeps the field one word.
pub fn properties(pairs: &[KeyValuePair]) -> String {
    if pairs.is_empty() {
        return "none".to_string();
    }
    let texts: Vec<String> = pairs
        .iter()
        .map(|pair| match pair.value() {
            Value::Varint(value) => format!("{}:{value}", pair.ty()),
            Value::Bytes(bytes) => format!("{}:{}", pair.ty(), hex::encode(bytes)),
        })
        .collect();
    texts.join(",")
}

/// An Object that belongs to no subscription this client holds.
pub fn ignored(track_alias: u64) {
    println!("IGNORED track_alias={track_alias}");
}

pub fn publish_done_received(done: &PublishDone) {
    println!(
        "PUBLISH_DONE status_code={:#x} stream_count={} reason={:?}",
        done.status_code,
        done.stream_count,
        String::from_utf8_lossy(done.error_reason.as_bytes()),
    );
}

/// The subscription is over before the requested number of Objects arrived.
pub fn subscription_ended(why: &str, received: u64, wanted: u64) {
    println!("SUBSCRIPTION_ENDED why={why} received={received} wanted={wanted}");
}

pub fn track_malformed(track_alias: u64, error: &MoqtError) {
    println!("TRACK_MALFORMED track_alias={track_alias} error={error}");
}

pub fn namespace_published() {
    println!("NAMESPACE_PUBLISHED");
}

pub fn published(track_alias: u64, forward: bool) {
    println!("PUBLISHED track_alias={track_alias} forward={forward}");
}

/// A `REQUEST_UPDATE` was answered with `REQUEST_ERROR` `NOT_SUPPORTED`.
pub fn update_refused() {
    println!("UPDATE_REFUSED");
}

/// How the peer ended its direction of the request stream after this client's
/// `PUBLISH_DONE`, or `None` when it did not in time.
///
/// WHY only what was seen, and no word like "confirmed": that a FIN means
/// "every stream the Stream Count named has ended here" holds for `XmoQ`'s
/// relay alone. A peer that follows only the draft may finish the stream for
/// any reason, and this client cannot tell which peer it talks to.
pub fn publish_done_peer_ended(how: Option<RequestClose>) {
    match how {
        Some(how) => println!("PUBLISH_DONE_PEER_ENDED how={how}"),
        None => println!("PUBLISH_DONE_PEER_ENDED how=nothing in time"),
    }
}

/// The same for a subscription the peer asked for with SUBSCRIBE, where this
/// client reads the peer's stream through its request readers and learns only
/// whether the stream ended with a FIN.
pub fn publish_done_peer_ended_on_its_stream(fin: Option<bool>) {
    let how = match fin {
        Some(true) => "FIN",
        Some(false) => "reset or stopped",
        None => "nothing in time",
    };
    println!("PUBLISH_DONE_PEER_ENDED how={how}");
}

/// The subscriber of this client's PUBLISH finished its direction of the
/// request stream before this client sent `PUBLISH_DONE`. The subscription
/// goes on (§5.1).
pub fn subscriber_finished_its_direction() {
    println!("PUBLISH_PEER_FINISHED_ITS_DIRECTION");
}

/// The peer subscribed to the track this client announced the namespace of.
pub fn serving(track_alias: u64) {
    println!("SERVING track_alias={track_alias}");
}

/// The subscriber ended that subscription (§5.1: `STOP_SENDING`, or a reset of
/// its own direction).
pub fn subscription_cancelled() {
    println!("SUBSCRIPTION_CANCELLED");
}

/// The peer sent `STOP_SENDING` for a stream this client writes: `what` says
/// which. `code=none`: the peer's code was outside the range WebTransport
/// gives applications, or there was none.
pub fn stop_sending_received(what: &str, code: Option<u32>) {
    match code {
        Some(code) => println!("STOP_SENDING_RECEIVED {what} code={code:#x}"),
        None => println!("STOP_SENDING_RECEIVED {what} code=none"),
    }
}

pub fn sent(group_id: u64, payload_len: usize) {
    println!("SENT group={group_id} object=0 payload_len={payload_len}");
}

/// The raw bytes of one write or one read, printed with `--trace`.
pub fn trace(direction: Direction, stream: &str, bytes: &[u8]) {
    let direction = match direction {
        Direction::Send => "send",
        Direction::Recv => "recv",
    };
    println!("TRACE {direction} {stream} {}", hex::encode(bytes));
}

/// A subgroup stream ended without a FIN: reset by the peer with this code,
/// stopped by this client, or lost.
pub fn stream_aborted(stream: u64, code: Option<u32>) {
    match code {
        Some(code) => println!("STREAM_ABORTED uni#{stream} code={code:#x}"),
        None => println!("STREAM_ABORTED uni#{stream}"),
    }
}
