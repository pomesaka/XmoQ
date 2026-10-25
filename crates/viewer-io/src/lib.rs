//! What a viewer of a room does on its sockets, for the viewer of the
//! terminal (`xmoq-watch`) and the reader for an agent (`xmoq-agent`): the
//! room's WebSocket ([`room_socket`]), the requests of a session and the
//! catalog ([`session`]), the chat on tracks ([`chat_tracks`]), and the
//! following of a track of one item a Group, the chat's `lines` and a
//! transcript's `speech` ([`follow`]).
//! What they decide is `xmoq-viewer`'s, where its tests are.
//!
//! WHY a crate of its own apart from `xmoq-viewer`, as `xmoq-moqt-io` is
//! apart from `xmoq-moqt`: the library of `xmoq-watch` touches no socket,
//! and that holds of what it shares as well. WHY NOT one crate with both:
//! I/O and what decides would be in one library. WHY NOT a copy in each
//! program: the two would drift.

pub mod chat_tracks;
pub mod follow;
pub mod reconnect;
pub mod room_socket;
pub mod session;
