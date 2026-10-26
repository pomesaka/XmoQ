//! What a viewer of a room does on its sockets, for the viewer of the
//! terminal (`xmoq-watch`) and the reader for an agent (`xmoq-agent`): the
//! room's HTTP API ([`room_client`]) and the chat on tracks
//! ([`chat_tracks`]). The tracks of others they read are `xmoq-subscriber`'s,
//! and the start and the end of a session `client` of `xmoq-moqt-io`'s, as
//! every client's. What they decide is `xmoq-viewer`'s, where its tests are.
//!
//! WHY a crate of its own apart from `xmoq-viewer`, as `xmoq-moqt-io` is
//! apart from `xmoq-moqt`: the library of `xmoq-watch` touches no socket,
//! and that holds of what it shares as well. WHY NOT one crate with both:
//! I/O and what decides would be in one library. WHY NOT a copy in each
//! program: the two would drift.

// FIXME: joining a room again and the wait before a try are written in
// both programs, each with the `unless_ended` of its own loop; one here
// once a wait that their signal ends is shared.

pub mod chat_tracks;
pub mod room_client;
