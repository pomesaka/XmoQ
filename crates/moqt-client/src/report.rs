//! Every line this client prints to stdout.
//!
//! The format of these lines is a contract with the scripts that drive the
//! client in interoperability runs: they wait for `SUBSCRIBED`, `PUBLISHED` and
//! `SENT`, and read the payload from `OBJECT`. The other lines are for people
//! reading a log. Keeping all of them here keeps that contract in one place.

use std::fmt::Display;

use xmoq_moqt::Error as MoqtError;
use xmoq_moqt::data::{SubgroupHeader, SubgroupObject};
use xmoq_moqt::message::{PublishDone, Setup};

use crate::hex;

/// Which way the bytes of a `TRACE` line travelled.
#[derive(Clone, Copy)]
pub enum Direction {
    Send,
    Recv,
}

pub fn connected(url: &str) {
    println!("CONNECTED url={url}");
}

pub fn setup_ok(peer_setup: &Setup) {
    println!("SETUP_OK server_setup={peer_setup:?}");
}

pub fn subscribed(track_alias: u64) {
    println!("SUBSCRIBED track_alias={track_alias}");
}

pub fn object(header: &SubgroupHeader, object: &SubgroupObject) {
    println!(
        "OBJECT group={} object={} status={:?} payload_len={} payload_hex={}",
        header.group_id,
        object.object_id,
        object.status,
        object.payload.len(),
        hex::encode(&object.payload),
    );
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

pub fn sent(payload_len: usize) {
    println!("SENT group=0 object=0 payload_len={payload_len}");
}

/// The raw bytes of one write or one read, printed with `--trace`.
pub fn trace(direction: Direction, stream: &str, bytes: &[u8]) {
    let direction = match direction {
        Direction::Send => "send",
        Direction::Recv => "recv",
    };
    println!("TRACE {direction} {stream} {}", hex::encode(bytes));
}

/// The peer reset a stream.
pub fn stream_reset(stream: &str, code: u64) {
    println!("STREAM_RESET {stream} code={code:#x}");
}

/// This client stopped reading a stream.
pub fn stream_stopped(stream: &str, why: impl Display) {
    println!("STREAM_STOPPED {stream} {why}");
}
