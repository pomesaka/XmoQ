//! Every line this client prints to stdout.
//!
//! The format of these lines is a contract with the scripts that drive the
//! client in interoperability runs: they wait for `SUBSCRIBED`, `PUBLISHED` and
//! `SENT`, and read the payload from `OBJECT`. The other lines are for people
//! reading a log. Keeping all of them here keeps that contract in one place.

use xmoq_moqt::Error as MoqtError;
use xmoq_moqt::data::{SubgroupHeader, SubgroupObject};
use xmoq_moqt::fetch::{FetchObject, RangeEnd};
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::message::{PublishDone, Setup};
use xmoq_moqt::name::Location;

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

/// This client sent `PUBLISH_DONE` and goes on with the session: the first
/// of the two steps of publishing a track anew.
pub fn publish_done_sent() {
    println!("PUBLISH_DONE_SENT");
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

/// `--publish-on-line`: the publisher has done everything before its PUBLISH
/// and waits for the driver's line `publish`.
pub fn ready_to_publish() {
    println!("READY_TO_PUBLISH");
}

/// `--serve-on-line`: the SUBSCRIBE this client would serve has come, with
/// this Request ID, and waits for the driver's line `serve`.
pub fn asked(request_id: u64) {
    println!("ASKED request_id={request_id}");
}

/// A Standalone FETCH for this client's track came, with this Request ID.
pub fn fetch_asked(request_id: u64, start: Location, end: Location) {
    println!(
        "FETCH_ASKED request_id={request_id} start_group={} start_object={} end_group={} end_object={}",
        start.group, start.object, end.group, end.object
    );
}

/// This client accepted that FETCH: the End Location of its `FETCH_OK`.
pub fn fetch_answered(request_id: u64, end_location: Location) {
    println!(
        "FETCH_ANSWERED request_id={request_id} end_group={} end_object={}",
        end_location.group, end_location.object
    );
}

/// This client refused that FETCH, with this code.
pub fn fetch_not_answered(request_id: u64, error_code: u64) {
    println!("FETCH_ANSWERED request_id={request_id} error_code={error_code:#x}");
}

/// Printed once for every FETCH this client accepted, whatever became of
/// it. `how` is one word; `code` is the code the subscriber ended it with,
/// where it did; `objects` counts the Objects written to their end.
pub fn fetch_served(request_id: u64, how: &str, code: Option<u32>, objects: u64) {
    let code = code.map_or_else(|| "none".to_string(), |code| format!("{code:#x}"));
    println!("FETCH_SERVED request_id={request_id} how={how} code={code} objects={objects}");
}

/// A FETCH of this client went out, with this Request ID.
pub fn fetch_sent(request_id: u64) {
    println!("FETCH_SENT request_id={request_id}");
}

pub fn fetch_ok(
    request_id: u64,
    end_location: Location,
    end_of_track: bool,
    track_properties: &[KeyValuePair],
) {
    println!(
        "FETCH_OK request_id={request_id} end_group={} end_object={} end_of_track={end_of_track} track_properties={}",
        end_location.group,
        end_location.object,
        properties(track_properties),
    );
}

/// The peer refused a FETCH of this client. `FETCH_ENDED` follows, as for
/// every FETCH.
pub fn fetch_refused(request_id: u64, error_code: u64) {
    println!("FETCH_REFUSED request_id={request_id} error_code={error_code:#x}");
}

/// `payload_hex` is the last field, as in `OBJECT`. WHY it has the first 64
/// bytes of the payload only, where `OBJECT` has all of it: a fetch stream is
/// asked for Objects of megabytes, to see a subscriber that reads slowly.
/// `payload_len` is the whole length.
pub fn fetch_object(request_id: u64, object: &FetchObject) {
    let shown = object.payload.get(..64).unwrap_or(&object.payload);
    println!(
        "FETCH_OBJECT request_id={request_id} group={} object={} subgroup={} properties={} payload_len={} payload_hex={}",
        object.group_id,
        object.object_id,
        object
            .subgroup_id
            .map_or_else(|| "none".to_string(), |id| id.to_string()),
        properties(&object.properties),
        object.payload.len(),
        hex::encode(shown),
    );
}

/// An End of Range indicator of a fetch stream (§11.4.4.2).
pub fn fetch_range_end(request_id: u64, kind: RangeEnd, location: Location) {
    let kind = match kind {
        RangeEnd::NonExistent => "non-existent",
        RangeEnd::Unknown => "unknown",
    };
    println!(
        "FETCH_RANGE_END request_id={request_id} kind={kind} group={} object={}",
        location.group, location.object
    );
}

/// A FETCH of this client is over: printed once for every FETCH that was
/// sent, whatever became of it. `how` is one word; `code` is the code of the
/// reset that ended it, where one did; `objects` counts the
/// `FETCH_OBJECT` lines printed for it.
pub fn fetch_ended(request_id: u64, how: &str, code: Option<u32>, objects: u64) {
    let code = code.map_or_else(|| "none".to_string(), |code| format!("{code:#x}"));
    println!("FETCH_ENDED request_id={request_id} how={how} code={code} objects={objects}");
}

/// What a fetch stream carried was refused, and the FETCH given up for it.
pub fn fetch_stream_refused(request_id: u64, error: &MoqtError) {
    println!("FETCH_STREAM_REFUSED request_id={request_id} error={error}");
}
