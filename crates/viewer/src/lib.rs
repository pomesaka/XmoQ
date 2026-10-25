//! What a viewer of a room decides, for the viewer of the terminal
//! (`xmoq-watch`) and the reader for an agent (`xmoq-agent`): what a room
//! says and is told ([`room`]), which lines of the chat on tracks are
//! fetched, shown and said ([`chat_track`]), who is there by the namespaces
//! the relay tells of ([`presence`]), and whose picture of a meeting is
//! taken ([`meeting`]). Nothing here reads a clock or a
//! socket; `xmoq-viewer-io` does, with these.
//!
//! WHY a crate of its own, and its I/O in another (`xmoq-viewer-io`), as
//! `xmoq-moqt` and `xmoq-moqt-io` are: the two programs share it, and what
//! decides stays where nothing touches a socket, with its tests. WHY NOT in
//! the library of `xmoq-watch`: a program that depends on it builds the
//! viewer's decoders and sound output (`cpal`, which needs ALSA on Linux).
//! WHY NOT a copy in each program: the two would drift.

pub mod chat_track;
pub mod meeting;
pub mod presence;
pub mod reconnect;
pub mod room;
