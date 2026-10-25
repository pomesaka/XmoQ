//! The tracks of the transcripts (decided on 2026-10-25): what
//! `xmoq-transcriber` publishes of the voices of the broadcasts it listens
//! to, as every side uses them: the names of the namespaces and tracks
//! ([`names`]) and the payload of a piece of speech ([`payloads`]).
//!
//! WHY a crate of its own, apart from the transcriber: an agent reads the
//! tracks (a page reads them with `@xmoq/transcript`), and the transcriber
//! builds whisper.cpp from C++ with cmake, which nothing that reads a
//! transcript should need.

pub mod names;
pub mod payloads;
