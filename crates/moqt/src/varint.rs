//! Variable-length integers (§1.4.1).
//!
//! This is not the QUIC varint of RFC 9000: the number of leading 1 bits of the
//! first byte gives the number of bytes that follow, and all 64-bit values fit.

/// Longest encoding, used for values of 2^56 and above.
pub const MAX_LEN: usize = 9;

/// Appends `value` in the shortest encoding.
pub fn encode(value: u64, out: &mut Vec<u8>) {
    let len = encoded_len(value);
    if len == MAX_LEN {
        out.push(0xff);
        out.extend_from_slice(&value.to_be_bytes());
        return;
    }
    // `len - 1` leading 1 bits mark the length; the value never reaches them
    // because `encoded_len` leaves `len` bits of headroom.
    let mut length_bits = !(0xffu8 >> (len - 1));
    for byte in value.to_be_bytes().into_iter().skip(8 - len) {
        out.push(byte | length_bits);
        length_bits = 0;
    }
}

/// Number of bytes `encode` writes for `value`.
#[must_use]
pub fn encoded_len(value: u64) -> usize {
    // A length of n bytes carries 7 * n bits, up to 8 bytes (Table 1).
    (1..MAX_LEN)
        .find(|len| value >> (7 * len) == 0)
        .unwrap_or(MAX_LEN)
}

/// Decodes one integer from the start of `buf`.
///
/// Returns the value and the number of bytes it used, or `None` when `buf` ends
/// before the integer does. Any encoding length is accepted, as the draft allows
/// non-minimal encodings.
#[must_use]
pub fn decode(buf: &[u8]) -> Option<(u64, usize)> {
    let first = *buf.first()?;
    let following = first.leading_ones();
    // At most 8, so the conversion cannot fail.
    let len = usize::try_from(following).ok()? + 1;
    let rest = buf.get(1..len)?;

    // The length bits and the 0 that ends them take `following + 1` bits. With 7
    // or 8 following bytes that is the whole first byte, which then carries no value.
    let value_mask = 0xffu8.checked_shr(following + 1).unwrap_or(0);
    let high_bits = u64::from(first & value_mask);
    let value = rest
        .iter()
        .fold(high_bits, |acc, byte| (acc << 8) | u64::from(*byte));
    Some((value, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table 2 of the draft, plus the edges of each length from Table 1.
    const MINIMAL: &[(&[u8], u64)] = &[
        (&[0x25], 37),
        (&[0xbb, 0xbd], 15_293),
        (&[0xed, 0x7f, 0x3e, 0x7d], 226_442_877),
        (&[0xfa, 0xa1, 0xa0, 0xe4, 0x03, 0xd8], 2_893_212_287_960),
        (
            &[0xfc, 0x89, 0x98, 0xab, 0xc6, 0x6b, 0xc0],
            151_288_809_941_952,
        ),
        (
            &[0xfe, 0xfa, 0x31, 0x8f, 0xa8, 0xe3, 0xca, 0x11],
            70_423_237_261_249_041,
        ),
        (
            &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            18_446_744_073_709_551_615,
        ),
        (&[0x00], 0),
        (&[0x7f], 127),
        (&[0x80, 0x80], 128),
        (&[0xbf, 0xff], 16_383),
        (&[0xc0, 0x40, 0x00], 16_384),
        (&[0xdf, 0xff, 0xff], 2_097_151),
        (&[0xe0, 0x20, 0x00, 0x00], 2_097_152),
        (&[0xf0, 0x10, 0x00, 0x00, 0x00], 268_435_456),
        (
            &[0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            72_057_594_037_927_935,
        ),
        (
            &[0xff, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            72_057_594_037_927_936,
        ),
        // Track Alias sent by moqtail-ts 0.13.0 in a PUBLISH, as logged by the
        // moqtail relay 0.15.0 (alias=779698128977928661).
        (
            &[0xff, 0x0a, 0xd2, 0x0b, 0x51, 0x39, 0xc4, 0x21, 0xd5],
            779_698_128_977_928_661,
        ),
    ];

    #[test]
    fn decodes_minimal_encodings() {
        for (bytes, value) in MINIMAL {
            assert_eq!(decode(bytes), Some((*value, bytes.len())), "{bytes:02x?}");
        }
    }

    #[test]
    fn encodes_minimally() {
        for (bytes, value) in MINIMAL {
            let mut out = Vec::new();
            encode(*value, &mut out);
            assert_eq!(out.as_slice(), *bytes, "{value}");
        }
    }

    #[test]
    fn decodes_non_minimal_encodings() {
        let cases: &[(&[u8], u64)] = &[
            // Table 2: 37 in two bytes.
            (&[0x80, 0x25], 37),
            (&[0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x25], 37),
        ];
        for (bytes, value) in cases {
            assert_eq!(decode(bytes), Some((*value, bytes.len())), "{bytes:02x?}");
        }
    }

    #[test]
    fn ignores_bytes_after_the_integer() {
        assert_eq!(decode(&[0x25, 0xff, 0xff]), Some((37, 1)));
    }

    #[test]
    fn reports_truncated_input() {
        let cases: &[&[u8]] = &[
            &[],
            &[0x80],
            &[0xc0, 0x00],
            &[0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ];
        for bytes in cases {
            assert_eq!(decode(bytes), None, "{bytes:02x?}");
        }
    }
}
