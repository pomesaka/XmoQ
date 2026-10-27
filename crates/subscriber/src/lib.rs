//! What a client of the relay needs to read the tracks of others on its
//! session, as the terminal viewer (`xmoq-watch`) and the reader for an
//! agent (`xmoq-agent`) do: a kept track, subscribed to and asked for again
//! by one rule ([`kept`]), a followed track of one numbered item a Group,
//! read from a point in its past ([`follow`], what it decides in
//! [`followed`]), the catalog of a broadcast that may not be there yet
//! ([`catalog`]), a `SUBSCRIBE_NAMESPACE` asked for again by the same rule
//! ([`namespaces`]), and the requests and the events of the session they go
//! by ([`session`]).
//!
//! The name of the catalog track and the most a catalog may be are
//! `xmoq-msf`'s. What a client of each role values (how many of the latest
//! items of a followed track it fetches, how many it catches up) is the
//! caller's: the chat's are `RECENT_LINES` and `CATCH_UP_LINES` of
//! `xmoq-viewer`.
//!
//! WHY a crate of its own, apart from the viewers: the gatherer of the chat
//! and the transcriber subscribe to tracks of others too, and a part that
//! needed this later could not depend on the viewers' and would write its
//! own (`docs/architecture.md`, "The roles of an endpoint"). WHY NOT in
//! `xmoq-moqt-io`: that crate reads and writes messages for any peer, and
//! these are the policy of a client that subscribes (its waits, when it asks
//! again, what it gives up).

pub mod catalog;
pub mod follow;
pub mod followed;
pub mod kept;
pub mod namespaces;
pub mod session;
