//! Track Namespace, Track Name and Location (§1.4.2, §2.4.1).

use crate::coding::{ReadResult, Reader, put_count, put_length_prefixed};
use crate::{Error, UsageError, varint};

/// Most Track Namespace Fields a Track Namespace may hold (§2.4.1).
pub const MAX_NAMESPACE_FIELDS: usize = 32;
/// Longest Full Track Name: the field lengths plus the Track Name length (§2.4.1).
pub const MAX_FULL_TRACK_NAME_LEN: usize = 4_096;

const TOO_MANY_FIELDS: &str = "more than 32 track namespace fields";
const FULL_TRACK_NAME_TOO_LONG: &str = "full track name exceeds 4096 bytes";

/// The position of an Object in a track (§1.4.2). Locations order by Group, then
/// by Object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    /// Group ID.
    pub group: u64,
    /// Object ID within the Group.
    pub object: u64,
}

impl Location {
    pub(crate) fn decode(reader: &mut Reader<'_>) -> ReadResult<Self> {
        Ok(Self {
            group: reader.varint()?,
            object: reader.varint()?,
        })
    }

    pub(crate) fn encode(self, out: &mut Vec<u8>) {
        varint::encode(self.group, out);
        varint::encode(self.object, out);
    }
}

/// An ordered list of 0 to 32 non-empty byte fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackNamespace(Vec<Vec<u8>>);

impl TrackNamespace {
    /// A Track Namespace of these fields.
    ///
    /// # Errors
    ///
    /// Fails for more than 32 fields, an empty field, or more than 4096 bytes in
    /// total (§2.4.1).
    pub fn new(fields: Vec<Vec<u8>>) -> Result<Self, UsageError> {
        check_namespace(&fields).map_err(UsageError)?;
        Ok(Self(fields))
    }

    /// The Track Namespace Fields, in order.
    #[must_use]
    pub fn fields(&self) -> &[Vec<u8>] {
        &self.0
    }

    /// Whether this namespace matches `other` by Namespace Prefix Matching
    /// (§9.5): its fields equal the first fields of `other`, field by field.
    /// `(foo)` and `(foo, bar)` match `(foo, bar)`; `(foobar)` does not.
    #[must_use]
    pub fn is_prefix_of(&self, other: &Self) -> bool {
        other.0.starts_with(&self.0)
    }

    /// The sum of the field lengths, which is what the 4096-byte limit counts.
    fn total_len(&self) -> usize {
        total_len(&self.0)
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> ReadResult<Self> {
        let count = reader.length()?;
        // Checked before the loop so that the field count the peer announces
        // never sizes an allocation.
        if count > MAX_NAMESPACE_FIELDS {
            return Err(Error::ProtocolViolation(TOO_MANY_FIELDS).into());
        }
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            let len = reader.length()?;
            fields.push(reader.bytes(len)?.to_vec());
        }
        // The same rules as `new`; what the caller may not build, the peer may
        // not send.
        check_namespace(&fields).map_err(Error::ProtocolViolation)?;
        Ok(Self(fields))
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        put_count(self.0.len(), out);
        for field in &self.0 {
            put_length_prefixed(field, out);
        }
    }
}

fn total_len(fields: &[Vec<u8>]) -> usize {
    fields
        .iter()
        .fold(0usize, |sum, field| sum.saturating_add(field.len()))
}

fn check_namespace(fields: &[Vec<u8>]) -> Result<(), &'static str> {
    if fields.len() > MAX_NAMESPACE_FIELDS {
        return Err(TOO_MANY_FIELDS);
    }
    if fields.iter().any(Vec::is_empty) {
        return Err("empty track namespace field");
    }
    if total_len(fields) > MAX_FULL_TRACK_NAME_LEN {
        return Err("track namespace exceeds 4096 bytes");
    }
    Ok(())
}

/// A Track Namespace and a Track Name, the identity of a track. Together they
/// are at most 4096 bytes (§2.4.1), which every value of this type satisfies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FullTrackName {
    namespace: TrackNamespace,
    name: Vec<u8>,
}

impl FullTrackName {
    /// A Full Track Name. The name may be empty.
    ///
    /// # Errors
    ///
    /// Fails when the namespace fields and the name together exceed 4096 bytes
    /// (§2.4.1).
    pub fn new(namespace: TrackNamespace, name: Vec<u8>) -> Result<Self, UsageError> {
        check_full_track_name(&namespace, &name).map_err(UsageError)?;
        Ok(Self { namespace, name })
    }

    /// The Track Namespace.
    #[must_use]
    pub fn namespace(&self) -> &TrackNamespace {
        &self.namespace
    }

    /// The Track Name.
    #[must_use]
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// Reads `Track Namespace, Track Name Length, Track Name`, the order every
    /// message in this crate uses.
    pub(crate) fn decode(reader: &mut Reader<'_>) -> ReadResult<Self> {
        let namespace = TrackNamespace::decode(reader)?;
        let name_len = reader.length()?;
        let name = reader.bytes(name_len)?;
        // The same rule as `new`; what the caller may not build, the peer may
        // not send.
        check_full_track_name(&namespace, name).map_err(Error::ProtocolViolation)?;
        Ok(Self {
            namespace,
            name: name.to_vec(),
        })
    }

    /// Appends `Track Namespace, Track Name Length, Track Name`.
    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        self.namespace.encode(out);
        put_length_prefixed(&self.name, out);
    }
}

fn check_full_track_name(
    track_namespace: &TrackNamespace,
    track_name: &[u8],
) -> Result<(), &'static str> {
    if track_namespace.total_len().saturating_add(track_name.len()) > MAX_FULL_TRACK_NAME_LEN {
        return Err(FULL_TRACK_NAME_TOO_LONG);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::parse_exact;

    fn namespace(fields: &[&[u8]]) -> TrackNamespace {
        TrackNamespace(fields.iter().map(|field| field.to_vec()).collect())
    }

    #[test]
    fn decodes_and_encodes_the_same_bytes() {
        // (bytes, namespace fields, track name)
        type Case<'a> = (&'a [u8], &'a [&'a [u8]], &'a [u8]);
        let cases: &[Case<'_>] = &[
            // ("xmoq", "phase0") / "object", as sent by moqtail-ts 0.13.0.
            (
                &[
                    0x02, 0x04, 0x78, 0x6d, 0x6f, 0x71, 0x06, 0x70, 0x68, 0x61, 0x73, 0x65, 0x30,
                    0x06, 0x6f, 0x62, 0x6a, 0x65, 0x63, 0x74,
                ],
                &[b"xmoq", b"phase0"],
                b"object",
            ),
            // No namespace fields and an empty name are both legal.
            (&[0x00, 0x00], &[], b""),
        ];
        for (bytes, fields, name) in cases {
            let expected = FullTrackName {
                namespace: namespace(fields),
                name: name.to_vec(),
            };
            assert_eq!(
                parse_exact(bytes, FullTrackName::decode),
                Ok(expected.clone()),
                "{bytes:02x?}"
            );
            let mut out = Vec::new();
            expected.encode(&mut out);
            assert_eq!(out.as_slice(), *bytes);
        }
    }

    #[test]
    fn rejects_malformed_namespaces() {
        let mut thirty_three_fields = vec![0x21];
        thirty_three_fields.extend([0x01, 0x61].repeat(33));
        let mut oversized_field = vec![0x01, 0x90, 0x01];
        oversized_field.extend(vec![0x61; 4097]);

        let cases: &[(&[u8], Error)] = &[
            (
                &[0x01, 0x00],
                Error::ProtocolViolation("empty track namespace field"),
            ),
            (
                &thirty_three_fields,
                Error::ProtocolViolation("more than 32 track namespace fields"),
            ),
            // One field of 4097 bytes.
            (
                &oversized_field,
                Error::ProtocolViolation("track namespace exceeds 4096 bytes"),
            ),
            (
                &[0x01, 0x02, 0x61],
                Error::ProtocolViolation("length is shorter than the fields it holds"),
            ),
        ];
        for (bytes, error) in cases {
            assert_eq!(
                parse_exact(bytes, TrackNamespace::decode),
                Err(error.clone()),
                "{:02x?}",
                &bytes[..bytes.len().min(8)]
            );
        }
    }

    #[test]
    fn rejects_a_full_track_name_over_the_limit() {
        // A 4096-byte namespace field followed by a 1-byte name.
        let mut bytes = vec![0x01, 0x90, 0x00];
        bytes.extend(vec![0x61; 4096]);
        bytes.extend([0x01, 0x62]);
        assert_eq!(
            parse_exact(&bytes, FullTrackName::decode),
            Err(Error::ProtocolViolation(
                "full track name exceeds 4096 bytes"
            ))
        );
    }

    #[test]
    fn constructors_enforce_the_same_limits() {
        assert_eq!(
            TrackNamespace::new(vec![vec![]]),
            Err(UsageError("empty track namespace field"))
        );
        assert_eq!(
            TrackNamespace::new(vec![vec![0x61]; 33]),
            Err(UsageError("more than 32 track namespace fields"))
        );
        assert_eq!(
            FullTrackName::new(namespace(&[&[0x61; 4096]]), vec![0x62]),
            Err(UsageError("full track name exceeds 4096 bytes"))
        );
    }

    #[test]
    fn locations_order_by_group_then_object() {
        let cases = [
            (
                Location {
                    group: 1,
                    object: 9,
                },
                Location {
                    group: 2,
                    object: 0,
                },
            ),
            (
                Location {
                    group: 2,
                    object: 0,
                },
                Location {
                    group: 2,
                    object: 1,
                },
            ),
        ];
        for (smaller, larger) in cases {
            assert!(smaller < larger, "{smaller:?} < {larger:?}");
        }
    }
}
