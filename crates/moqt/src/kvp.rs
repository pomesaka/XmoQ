//! Key-Value-Pairs (§1.4.3), the serialization of Setup Options and Properties.

use crate::coding::{ReadResult, Reader, put_length_prefixed};
use crate::{Error, UsageError, varint};

/// Largest byte value a Key-Value-Pair may carry (§1.4.3).
pub const MAX_VALUE_LEN: usize = 65_535;

/// Most pairs decoded from one list.
///
/// WHY a count on top of the byte limits of the callers: a pair takes two bytes
/// on the wire and far more as a value in memory, so a list of tiny pairs would
/// grow to many times the bytes that were allowed in.
///
/// Provisional: the draft sets no limit, and `XmoQ` has not decided one. No
/// message or Object seen from moqtail carries more than a handful.
pub const MAX_PAIRS: usize = 64;

/// Property types reserved for Mandatory Track Properties (§2.5.1).
const MANDATORY_TRACK_PROPERTIES: std::ops::RangeInclusive<u64> = 0x4000..=0x7fff;

/// The value of a Key-Value-Pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// The value of an even type.
    Varint(u64),
    /// The value of an odd type, at most [`MAX_VALUE_LEN`] bytes.
    Bytes(Vec<u8>),
}

/// One pair. The parity of the type fixes the kind of value: even types carry a
/// varint, odd types carry bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValuePair {
    ty: u64,
    value: Value,
}

impl KeyValuePair {
    /// A pair of an even type.
    ///
    /// # Errors
    ///
    /// Fails for an odd type.
    pub fn varint(ty: u64, value: u64) -> Result<Self, UsageError> {
        if !ty.is_multiple_of(2) {
            return Err(UsageError("an odd type carries bytes"));
        }
        Ok(Self {
            ty,
            value: Value::Varint(value),
        })
    }

    /// A pair of an odd type.
    ///
    /// # Errors
    ///
    /// Fails for an even type or a value over [`MAX_VALUE_LEN`] bytes.
    pub fn bytes(ty: u64, value: Vec<u8>) -> Result<Self, UsageError> {
        if ty.is_multiple_of(2) {
            return Err(UsageError("an even type carries a varint"));
        }
        if value.len() > MAX_VALUE_LEN {
            return Err(UsageError("value longer than 65535 bytes"));
        }
        Ok(Self {
            ty,
            value: Value::Bytes(value),
        })
    }

    /// The Type: a Setup Option type or a Property type, depending on where the
    /// pair appears.
    #[must_use]
    pub fn ty(&self) -> u64 {
        self.ty
    }

    /// The value, whose kind follows from the parity of [`Self::ty`].
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Whether the type lies in the range of Mandatory Track Properties, which an
    /// endpoint must understand to use the track and which must not appear on an
    /// Object (§2.5.1).
    #[must_use]
    pub fn is_mandatory_track_property(&self) -> bool {
        MANDATORY_TRACK_PROPERTIES.contains(&self.ty)
    }
}

/// Decodes pairs until `reader` is exhausted. The caller bounds the reader to the
/// region that holds the pairs; the pairs themselves carry no count.
///
/// More than [`MAX_PAIRS`] pairs are [`Error::TooManyKeyValuePairs`], which ends
/// the session. A caller decoding the Properties of an Object turns that into
/// an error that ends only the stream.
pub(crate) fn decode_all(reader: &mut Reader<'_>) -> ReadResult<Vec<KeyValuePair>> {
    let mut pairs = Vec::new();
    let mut previous_type = 0u64;
    while !reader.is_empty() {
        if pairs.len() == MAX_PAIRS {
            return Err(Error::TooManyKeyValuePairs { limit: MAX_PAIRS }.into());
        }
        let delta = reader.varint()?;
        let ty = previous_type
            .checked_add(delta)
            .ok_or(Error::ProtocolViolation("key-value type exceeds 64 bits"))?;
        let value = if ty.is_multiple_of(2) {
            Value::Varint(reader.varint()?)
        } else {
            let len = reader.length()?;
            if len > MAX_VALUE_LEN {
                return Err(Error::ProtocolViolation("key-value length exceeds 65535").into());
            }
            Value::Bytes(reader.bytes(len)?.to_vec())
        };
        pairs.push(KeyValuePair { ty, value });
        previous_type = ty;
    }
    Ok(pairs)
}

/// Appends `pairs` in ascending type order, which the delta encoding requires.
/// Pairs of the same type keep their relative order.
pub(crate) fn encode_all(pairs: &[KeyValuePair], out: &mut Vec<u8>) {
    let mut ordered: Vec<&KeyValuePair> = pairs.iter().collect();
    ordered.sort_by_key(|pair| pair.ty);

    let mut previous_type = 0u64;
    for pair in ordered {
        // Sorted ascending above, so the difference never goes below zero.
        varint::encode(pair.ty.saturating_sub(previous_type), out);
        match &pair.value {
            Value::Varint(value) => varint::encode(*value, out),
            Value::Bytes(value) => put_length_prefixed(value, out),
        }
        previous_type = pair.ty;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::parse_exact;

    fn decode(bytes: &[u8]) -> Result<Vec<KeyValuePair>, Error> {
        parse_exact(bytes, decode_all)
    }

    fn encode(pairs: &[KeyValuePair]) -> Vec<u8> {
        let mut out = Vec::new();
        encode_all(pairs, &mut out);
        out
    }

    #[test]
    fn decodes_and_encodes_the_same_bytes() {
        let cases: &[(&[u8], &[KeyValuePair])] = &[
            (&[], &[]),
            // Setup Options of the moqtail relay 0.15.0, from its log:
            // MOQT_IMPLEMENTATION (0x07) = "moqtail/0.15.0+dd8623d".
            (
                &[
                    0x07, 0x16, 0x6d, 0x6f, 0x71, 0x74, 0x61, 0x69, 0x6c, 0x2f, 0x30, 0x2e, 0x31,
                    0x35, 0x2e, 0x30, 0x2b, 0x64, 0x64, 0x38, 0x36, 0x32, 0x33, 0x64,
                ],
                &[KeyValuePair {
                    ty: 0x07,
                    value: Value::Bytes(b"moqtail/0.15.0+dd8623d".to_vec()),
                }],
            ),
            // Types 4, 4 and 9: a repeated type has delta 0.
            (
                &[0x04, 0x25, 0x00, 0x26, 0x05, 0x02, 0xab, 0xcd],
                &[
                    KeyValuePair {
                        ty: 4,
                        value: Value::Varint(37),
                    },
                    KeyValuePair {
                        ty: 4,
                        value: Value::Varint(38),
                    },
                    KeyValuePair {
                        ty: 9,
                        value: Value::Bytes(vec![0xab, 0xcd]),
                    },
                ],
            ),
        ];
        for (bytes, pairs) in cases {
            assert_eq!(decode(bytes).as_deref(), Ok(*pairs), "{bytes:02x?}");
            assert_eq!(encode(pairs).as_slice(), *bytes, "{pairs:?}");
        }
    }

    #[test]
    fn encodes_in_ascending_type_order() {
        let pairs = [
            KeyValuePair::bytes(9, vec![0xab]).unwrap(),
            KeyValuePair::varint(4, 37).unwrap(),
        ];
        assert_eq!(encode(&pairs), [0x04, 0x25, 0x05, 0x01, 0xab]);
    }

    #[test]
    fn rejects_malformed_input() {
        let cases: &[(&[u8], Error)] = &[
            // Odd type 1 announces 2 bytes and holds 1.
            (
                &[0x01, 0x02, 0xab],
                Error::ProtocolViolation("length is shorter than the fields it holds"),
            ),
            // Even type 2 with no value.
            (
                &[0x02],
                Error::ProtocolViolation("length is shorter than the fields it holds"),
            ),
            // Type 2^64-1, then a delta of 1.
            (
                &[
                    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x01, 0x00,
                ],
                Error::ProtocolViolation("key-value type exceeds 64 bits"),
            ),
            // Odd type 1 with length 65536.
            (
                &[0x01, 0xc1, 0x00, 0x00],
                Error::ProtocolViolation("key-value length exceeds 65535"),
            ),
        ];
        for (bytes, error) in cases {
            assert_eq!(decode(bytes), Err(error.clone()), "{bytes:02x?}");
        }
    }

    #[test]
    fn bounds_the_number_of_pairs() {
        // Type 0 with the value 0, over and over: two bytes a pair.
        let at_the_limit = [0x00, 0x00].repeat(64);
        let one_more = [0x00, 0x00].repeat(65);
        assert_eq!(decode(&at_the_limit).map(|pairs| pairs.len()), Ok(64));
        assert_eq!(
            decode(&one_more),
            Err(Error::TooManyKeyValuePairs { limit: 64 })
        );
    }

    #[test]
    fn constructors_enforce_type_parity_and_length() {
        assert_eq!(
            KeyValuePair::varint(7, 1),
            Err(UsageError("an odd type carries bytes"))
        );
        assert_eq!(
            KeyValuePair::bytes(7, vec![0; 65_536]),
            Err(UsageError("value longer than 65535 bytes"))
        );
        assert_eq!(
            KeyValuePair::bytes(8, vec![]),
            Err(UsageError("an even type carries a varint"))
        );
    }
}
