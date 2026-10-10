//! WebTransport application error codes on HTTP/3.
//!
//! A WebTransport stream is a QUIC stream of an HTTP/3 connection, so the code
//! of a `RESET_STREAM` or `STOP_SENDING` frame is an HTTP/3 error code.
//! `draft-ietf-webtrans-http3-16` §4.4 reserves a range of those for the 32-bit
//! codes of the application and requires implementations to remap ("MUST remap
//! those error codes into the error range reserved for `WT_APPLICATION_ERROR`").
//!
//! WHY here and not in the transport library: `wtransport` 0.7.2 passes the
//! number it is given straight into the QUIC frame. Checked on 2026-10-10 with
//! Chromium 156.0.8078.4: a stream reset with the bare code 0x12 arrives in the
//! page as `streamErrorCode` 0, and one reset with `to_http3(0x12)` arrives as
//! 18 (`spikes/wt-error-codes`).

/// `0x00000000` of the application maps to this HTTP/3 error code (§4.4).
const FIRST: u64 = 0x52e4_a40f_a8db;
/// `0xffffffff` of the application maps to this one.
const LAST: u64 = 0x52e5_ac98_3162;

/// The HTTP/3 error code that carries an application error code:
/// `webtransport_code_to_http_code` of §4.4, Figure 4.
///
/// Every 0x1f-th codepoint of HTTP/3 is reserved (those of the form
/// `0x1f * N + 0x21`), so the mapping skips one after every 0x1e codes.
#[must_use]
pub fn to_http3(code: u32) -> u64 {
    let code = u64::from(code);
    FIRST + code + code / 0x1e
}

/// The application error code an HTTP/3 error code carries:
/// `http_code_to_webtransport_code` of §4.4, Figure 4.
///
/// `None` for a value outside the reserved range or on one of its reserved
/// codepoints: such a value carries no application code, for instance from a
/// peer that does not remap, or an HTTP/3 error of the peer's stack. §4.4: "the
/// stream is still considered reset, but the error code is not mapped to a
/// WebTransport application error code. The WebTransport implementation SHOULD
/// deliver this to the application as a stream reset with no application error
/// code." `draft-ietf-moq-transport-18` §14 then has the receiver treat it like
/// `INTERNAL_ERROR`.
#[must_use]
pub fn from_http3(http3_code: u64) -> Option<u32> {
    if !(FIRST..=LAST).contains(&http3_code) || (http3_code - 0x21).is_multiple_of(0x1f) {
        return None;
    }
    let shifted = http3_code - FIRST;
    u32::try_from(shifted - shifted / 0x1f).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_application_codes_into_the_reserved_range() {
        // (application code, HTTP/3 error code)
        let cases: [(u32, u64); 8] = [
            (0x0000_0000, 0x52e4_a40f_a8db),
            (0x0000_0001, 0x52e4_a40f_a8dc),
            // MALFORMED_TRACK of MoQ Transport; seen from Chromium as
            // 91141958510829 when a page aborts a stream with 0x12.
            (0x0000_0012, 91_141_958_510_829),
            // The last code before the first reserved codepoint, and the first
            // after it.
            (0x0000_001d, 0x52e4_a40f_a8f8),
            (0x0000_001e, 0x52e4_a40f_a8fa),
            (0x0000_003b, 0x52e4_a40f_a917),
            (0x0000_003c, 0x52e4_a40f_a919),
            (0xffff_ffff, 0x52e5_ac98_3162),
        ];
        for (code, http3_code) in cases {
            assert_eq!(to_http3(code), http3_code, "{code:#x}");
            assert_eq!(from_http3(http3_code), Some(code), "{http3_code:#x}");
        }
    }

    #[test]
    fn values_that_carry_no_application_code_map_to_none() {
        let cases: [u64; 7] = [
            // A bare application code, as a peer that does not remap sends it.
            0x12,
            // H3_NO_ERROR.
            0x100,
            // Just outside the range on either side.
            0x52e4_a40f_a8da,
            0x52e5_ac98_3163,
            // Reserved codepoints inside it (0x1f * N + 0x21).
            0x52e4_a40f_a8f9,
            0x52e4_a40f_a918,
            u64::MAX,
        ];
        for http3_code in cases {
            assert_eq!(from_http3(http3_code), None, "{http3_code:#x}");
        }
    }
}
