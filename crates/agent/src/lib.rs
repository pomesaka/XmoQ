//! A reader of a room for an agent: what happens in the room, one event a
//! line of JSON ([`events`]), and lines of stdin said in its chat or read as
//! commands ([`input`]). Who is in the chat is [`presence`]'s, and what the
//! flags mean [`args`]'s.
//!
//! This is the part that touches nothing, with the tests; the binary
//! (`src/bin/xmoq-agent/`) does the input and output, with
//! `xmoq-viewer-io` for the room, the session and the chat on tracks.

pub mod args;
pub mod events;
pub mod input;
pub mod presence;
pub mod transcript;
