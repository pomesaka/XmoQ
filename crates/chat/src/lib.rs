//! The gatherer of the chat: it is told of every participant of every room
//! (`SUBSCRIBE_NAMESPACE` for `xmoq/chat`), takes what each says on its
//! track `said`, numbers the lines of a room in the order they reach it,
//! keeps them, and publishes them on the track `lines` of the room; it
//! counts the reactions of a room (`reacted`) and how many are in it, and
//! publishes them on `reactions` and `present`.
//!
//! This is the part that touches nothing: whether and when a participant is asked again for its `said` ([`said_again`]), how fast one person may say lines
//! ([`rate`]), the numbering of a room ([`rooms`]), what is decided of
//! a line that comes ([`gather`]), what becomes of a reaction ([`react`]),
//! and what the flags mean ([`args`]). Each
//! is given its times and its bytes, and none reads a clock, a socket or the
//! store. The names and payloads of the tracks, and the limits of a line,
//! are `xmoq-chat-tracks`, which the other sides of the chat use too; how
//! long to wait before asking again and what a FETCH gets are
//! `xmoq-publisher`'s.

pub mod args;
pub mod gather;
pub mod rate;
pub mod react;
pub mod rooms;
pub mod said_again;
