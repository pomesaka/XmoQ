//! The tracks of the chat, as every side of it uses them: the names of the namespaces and tracks ([`names`]), the payloads of
//! `said` and `lines` ([`payloads`]) and of `reacted`, `reactions` and
//! `present` ([`reactions`]), and the limits of a line ([`limits`]).
//!
//! WHY a crate of its own, apart from the gatherer (`xmoq-chat`): the
//! terminal viewer speaks on these tracks too, and the gatherer's store
//! (`rusqlite`, which builds SQLite from C) is nothing a viewer should need
//! a C compiler for.

pub mod limits;
pub mod names;
pub mod payloads;
pub mod reactions;
