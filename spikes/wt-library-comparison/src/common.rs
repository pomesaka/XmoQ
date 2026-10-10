//! Values the two servers and the client share, so that a difference in a
//! result comes from the library and not from the probe.

#![allow(dead_code, reason = "each binary uses a part of this module")]

use std::time::Duration;

use anyhow::Result;
// Both libraries re-export the same quinn (0.11.12 in Cargo.lock).
use web_transport_quinn::quinn;
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{ControlMessage, setup_option};
use xmoq_moqt::session::{Role, Session, Transport};

pub const SESSION_CODE: u32 = 0x3;
pub const SESSION_REASON: &[u8] = b"protocol violation";
pub const RESET_CODE: u32 = 0x12;
pub const STOP_CODE: u32 = 0x5;

/// Unidirectional streams the peer may have open at once. HTTP/3 takes three of
/// them (control, QPACK encoder, QPACK decoder), so a browser gets five.
pub const MAX_UNI_STREAMS: u32 = 8;
pub const STREAM_RECEIVE_WINDOW: u32 = 64 * 1024;
pub const RECEIVE_WINDOW: u32 = 128 * 1024;

/// How long `/streams` and `/window` leave the peer unread before they report.
pub const HOLD: Duration = Duration::from_millis(1500);

/// The QUIC transport settings both servers run with.
pub fn transport_config() -> quinn::TransportConfig {
    let mut transport = quinn::TransportConfig::default();
    transport
        .max_concurrent_uni_streams(MAX_UNI_STREAMS.into())
        .stream_receive_window(STREAM_RECEIVE_WINDOW.into())
        .receive_window(RECEIVE_WINDOW.into())
        .keep_alive_interval(Some(Duration::from_secs(3)));
    transport
}

/// A MOQT session in the given role, and the bytes of its SETUP.
pub fn moqt_setup(role: Role) -> Result<(Session, Vec<u8>)> {
    let mut session = Session::new(role, Transport::WebTransport);
    let setup = session.setup(vec![KeyValuePair::bytes(
        setup_option::MOQT_IMPLEMENTATION,
        concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"))
            .as_bytes()
            .to_vec(),
    )?])?;
    Ok((session, encode(&setup)?))
}

pub fn encode(message: &ControlMessage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    message.encode(&mut bytes)?;
    Ok(bytes)
}

pub fn parse_sha256(hex: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(hex.len() == 64, "the certificate hash must be 64 hex digits");
    (0..32)
        .map(|index| Ok(u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)?))
        .collect()
}
