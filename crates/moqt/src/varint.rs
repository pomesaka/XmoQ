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

    /// The rows of `fixtures/moqt-18-by-hand/varints.tsv`, which
    /// `@xmoq/moqt` reads as well: Table 2 of §1.4.1 and the edges of each
    /// length of Table 1, with whether an encoder writes those bytes for the
    /// value (the fewest; §1.4.1: "any encoding length that can represent
    /// the value is valid").
    fn shared_varints() -> Vec<(Vec<u8>, u64, bool)> {
        include_str!("../../../fixtures/moqt-18-by-hand/varints.tsv")
            .lines()
            .map(|row| match row.split('\t').collect::<Vec<_>>()[..] {
                [hex, value, fewest] => (
                    (0..hex.len())
                        .step_by(2)
                        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                        .collect(),
                    value.parse().unwrap(),
                    match fewest {
                        "yes" => true,
                        "no" => false,
                        other => panic!("{other}"),
                    },
                ),
                _ => panic!("a row of three columns: {row}"),
            })
            .collect()
    }

    #[test]
    fn varints_are_read_and_written_as_the_shared_table_says() {
        let rows = shared_varints();
        // A table that lost rows would pass.
        assert_eq!(rows.len(), 26);
        for (bytes, value, fewest) in rows {
            assert_eq!(decode(&bytes), Some((value, bytes.len())), "{bytes:02x?}");
            let mut out = Vec::new();
            encode(value, &mut out);
            assert_eq!(out == bytes, fewest, "{value}: {out:02x?}");
            if fewest {
                assert_eq!(encoded_len(value), bytes.len(), "{value}");
            }
        }
    }

    #[test]
    fn a_track_alias_of_nine_bytes_from_another_implementation_is_read() {
        // The Track Alias sent by moqtail-ts 0.13.0 in a PUBLISH, as logged
        // by the moqtail relay 0.15.0 (alias=779698128977928661).
        let bytes = [0xff, 0x0a, 0xd2, 0x0b, 0x51, 0x39, 0xc4, 0x21, 0xd5];
        assert_eq!(decode(&bytes), Some((779_698_128_977_928_661, 9)));
        let mut out = Vec::new();
        encode(779_698_128_977_928_661, &mut out);
        assert_eq!(out, bytes);
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
