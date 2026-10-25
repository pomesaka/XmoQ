//! What a client of the relay needs to serve tracks of its own on a session
//! that it also subscribes on, as the gatherer of the chat (`xmoq-chat`) and
//! the transcriber (`xmoq-transcriber`) do. Such a client is told of namespaces under a
//! prefix, subscribes to a track of each, publishes one track of one Object
//! a Group for each prefix, and answers the relay's FETCH for it.
//!
//! The parts: how long to wait before asking again ([`backoff`]), what a
//! FETCH gets of such a track ([`fetch`]), the FETCH requests being answered
//! ([`answers`]), the PUBLISH of a track ([`publication`]; these four
//! without input or output), the requests the relay opens ([`requests`]),
//! and the tasks that write the streams ([`streams`]).
//!
//! WHY a crate of its own, apart from the gatherer: a transcriber of the
//! broadcasts, and an agent in a meeting, serve tracks the same way, and
//! would have had a copy of the gatherer's streams and rules. WHY NOT in `xmoq-moqt-io`: that crate reads and writes
//! messages for any peer, and these tasks are the policy of a client that
//! publishes (its waits, its limits, a Group a stream).

pub mod answers;
pub mod backoff;
pub mod fetch;
pub mod publication;
pub mod requests;
pub mod streams;

use std::time::Instant;

use xmoq_moqt::name::TrackNamespace;

/// A namespace for the log: its fields between slashes, each shown as Rust
/// shows a string.
#[must_use]
pub fn shown(namespace: &TrackNamespace) -> String {
    let fields: Vec<String> = namespace
        .fields()
        .iter()
        .map(|field| String::from_utf8_lossy(field).escape_debug().to_string())
        .collect();
    fields.join("/")
}

/// Completes at `at`; never without it.
pub async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => std::future::pending().await,
    }
}
