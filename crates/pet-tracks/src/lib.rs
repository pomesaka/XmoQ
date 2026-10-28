//! The tracks of a room's garden, as every side of it uses them: the names
//! of the namespaces and tracks ([`names`]) and the payloads of `garden`,
//! `acted` and `state` ([`payloads`]). `@xmoq/pet` has the same, and the
//! tables of `fixtures/pet-tracks/` pin both.
//!
//! WHY a crate of its own: the names and payloads of each kind of a room's
//! tracks are in a crate of the same form (`xmoq-chat-tracks`,
//! `xmoq-meeting-tracks`), with no input or output and nothing to build in
//! C, that the garden, the pet, a viewer and an agent can all depend on.

pub mod names;
pub mod payloads;
