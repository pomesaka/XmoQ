//! A reader of a room for an agent: what happens in the room, one event a
//! line of JSON ([`events`]) in files of `events/` named by
//! [`event_files`], and lines of stdin and of `inbox.txt` ([`inbox`]) said
//! in its chat or read as commands ([`input`]), and what the agent is given
//! to read there ([`guide`]). Who is in the chat is
//! [`presence`]'s, which transcript is followed [`transcript`]'s, and what
//! the flags mean [`args`]'s.
//!
//! This is the part that touches nothing, with the tests; the binary
//! (`src/bin/xmoq-agent/`) does the input and output, with
//! `xmoq-viewer-io` for the room, the session and the chat on tracks.

pub mod args;
pub mod event_files;
pub mod events;
pub mod guide;
pub mod inbox;
pub mod input;
pub mod presence;
pub mod transcript;
