//! `MoQ` Transport wire coding and session state for
//! `draft-ietf-moq-transport-18` (WebTransport subprotocol and ALPN `moqt-18`).
//!
//! The crate does no I/O. Decoders take bytes and return messages or objects;
//! state machines take messages and return events. A caller owns the QUIC or
//! WebTransport connection and moves bytes between it and these types, so the same
//! code serves a client, a relay and tests.
//!
//! Section numbers in comments (`§10.7`) refer to that draft.
//!
//! Only part of the draft is implemented. A message or stream type that the draft
//! defines and this crate does not decode is reported as
//! [`Error::UnsupportedMessageType`] or [`Error::UnsupportedStreamType`], never
//! mistaken for an unknown one.

mod aliases;
mod coding;
pub mod data;
mod error;
pub mod kvp;
pub mod message;
pub mod name;
pub mod params;
pub mod session;
#[cfg(test)]
mod test_util;
pub mod varint;

pub use error::{Error, ErrorScope, RequestFault, TerminationCode, UsageError, stream_reset_code};

/// Version identifier offered in `WT-Available-Protocols` and as the QUIC ALPN (§3.1).
pub const PROTOCOL: &str = "moqt-18";
