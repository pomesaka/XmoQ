//! What a viewer of a room does on its sockets, for the viewer of the
//! terminal (`xmoq-watch`) and the reader for an agent (`xmoq-agent`): the
//! room's WebSocket ([`room_socket`]), the requests of a session and the
//! catalog ([`session`]), and the chat on tracks ([`chat_tracks`]).
//! What they decide is `xmoq-viewer`'s, where its tests are.
//!
//! WHY a crate of its own apart from `xmoq-viewer`, as `xmoq-moqt-io` is
//! apart from `xmoq-moqt`: the library of `xmoq-watch` touches no socket,
//! and that holds of what it shares as well. WHY NOT one crate with both:
//! I/O and what decides would be in one library. WHY NOT a copy in each
//! program: the two would drift.

pub mod chat_tracks;
pub mod room_socket;
pub mod session;
