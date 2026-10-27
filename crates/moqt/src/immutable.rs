//! Immutable Properties (§12.7), a Track or Object Property whose value is a
//! sequence of Key-Value-Pairs: "When looking for the value of a property,
//! processors MUST search both the mutable properties and the contents of
//! Immutable Properties." `propertiesToSearch` of `@xmoq/moqt` reads them by
//! the same rule; both read the rows of `fixtures/immutable-properties/`.

use std::borrow::Cow;

use crate::Error;
use crate::coding::{ByteReader, ReadError};
use crate::kvp::{self, KeyValuePair, Value};

/// Property Type of Immutable Properties (§12.7).
pub const IMMUTABLE_PROPERTIES: u64 = 0x0b;

/// What the properties belong to: an Object may have one Immutable
/// Properties, a Track any number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Of {
    /// Object Properties.
    Object,
    /// Track Properties.
    Track,
}

/// The properties to look for the value of a property in, for a reader
/// above the session: those of [`properties_to_search`], in its order.
///
/// Properties that are malformed or cannot be read never get to such a
/// reader: the session refuses them first (`decode_track_properties`, the
/// [`TrackRefusal`] of a request, and the check of an Object's properties
/// in [`crate::data`]). For such properties this gives the ones outside
/// Immutable Properties, as they came. Borrowed where there are no
/// Immutable Properties, which is what most properties are.
#[must_use]
pub fn searched_properties(properties: &[KeyValuePair], of: Of) -> Cow<'_, [KeyValuePair]> {
    if properties
        .iter()
        .all(|property| property.ty() != IMMUTABLE_PROPERTIES)
    {
        return Cow::Borrowed(properties);
    }
    match properties_to_search(properties, of) {
        Ok(Ok(searched)) => Cow::Owned(searched),
        Ok(Err(_)) | Err(_) => Cow::Owned(
            properties
                .iter()
                .filter(|property| property.ty() != IMMUTABLE_PROPERTIES)
                .cloned()
                .collect(),
        ),
    }
}

/// [`searched_properties`] as a format above `MoQ` Transport reads them
/// (the Timestamp of LOC): each property's type and, for one that carries
/// an integer, its value; `None` for one that carries bytes.
#[must_use]
pub fn searched_integers(properties: &[KeyValuePair], of: Of) -> Vec<(u64, Option<u64>)> {
    searched_properties(properties, of)
        .iter()
        .map(|pair| {
            let value = match pair.value() {
                crate::kvp::Value::Varint(value) => Some(*value),
                crate::kvp::Value::Bytes(_) => None,
            };
            (pair.ty(), value)
        })
        .collect()
}

/// The properties §12.7 has a processor search: those without Immutable
/// Properties, in the order they came, followed by the pairs inside each
/// Immutable Properties. `Ok(Err(why))` where the Track is malformed (§2.4.2).
///
/// - An Object with more than one: "An Object MUST NOT contain more than one
///   instance of this property". The text gives no consequence; malformed,
///   since §2.4.2 has a Track that "does not conform to MOQT constraints"
///   malformed. A Track may have more than one: §12.7 does not forbid it, and
///   the contents of each are searched.
/// - Immutable Properties inside Immutable Properties: malformed (§12.7, "An
///   Object contains an Immutable Properties property that contains another
///   Immutable Properties key"). The text names an Object; **the same for a
///   Track**, since a second level is one §12.7 does not have searched.
/// - A pair inside that runs past the end of the value: malformed (§12.7, "A
///   Key-Value-Pair cannot be parsed").
///
/// The pairs inside are a sequence of their own (Figure 2): the first Delta
/// Type counts from 0, not from 0xB, as `moqtail` 0.15.0 reads them
/// (`src/model/property/object_property.rs`, read on 2026-10-24).
///
/// # Errors
///
/// What decoding a sequence fails with otherwise: the `PROTOCOL_VIOLATION`
/// of §1.4.3 (a Type over 2^64 - 1, a Length over 65,535), and
/// [`Error::TooManyKeyValuePairs`] past the local limit of one sequence,
/// which each Immutable Properties has for itself. WHY NOT malformed for the
/// first: §1.4.3 has "the Session MUST be closed with a `PROTOCOL_VIOLATION`"
/// of every Key-Value-Pair, and §12.7 makes no exception of the ones inside.
pub(crate) fn properties_to_search(
    properties: &[KeyValuePair],
    of: Of,
) -> Result<Result<Vec<KeyValuePair>, &'static str>, Error> {
    let (immutable, mut searched): (Vec<&KeyValuePair>, Vec<&KeyValuePair>) = properties
        .iter()
        .partition(|property| property.ty() == IMMUTABLE_PROPERTIES);
    if of == Of::Object && immutable.len() > 1 {
        return Ok(Err("more than one immutable properties on an object"));
    }
    let mut inside = Vec::new();
    for property in immutable {
        // An odd Type carries bytes (§1.4.3).
        let Value::Bytes(value) = property.value() else {
            continue;
        };
        let pairs = match kvp::decode_all(&mut ByteReader::new(value)) {
            Ok(pairs) => pairs,
            Err(ReadError::Short) => {
                return Ok(Err(
                    "a key-value pair inside immutable properties cannot be parsed",
                ));
            }
            Err(ReadError::Invalid(error)) => return Err(error),
        };
        if pairs.iter().any(|pair| pair.ty() == IMMUTABLE_PROPERTIES) {
            return Ok(Err("immutable properties inside immutable properties"));
        }
        inside.extend(pairs);
    }
    searched.extend(inside.iter());
    Ok(Ok(searched.into_iter().cloned().collect()))
}

/// Why the Track Properties of a `SUBSCRIBE_OK`, `FETCH_OK` or PUBLISH keep
/// this endpoint from processing the track, if they do: Immutable Properties
/// that make it malformed (§12.7), or a Mandatory Track Property, outside or
/// inside them (§2.5.1), of which this crate understands none. Malformed is
/// decided first: what is inside is not known then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackRefusal {
    /// The Track is malformed (§2.4.2, §12.7).
    Malformed,
    /// The Type of the first Mandatory Track Property searched.
    Unsupported(u64),
}

/// The [`TrackRefusal`] of Track Properties, if any. Properties whose
/// reading fails otherwise were refused when the message was decoded.
pub(crate) fn track_properties_refusal(properties: &[KeyValuePair]) -> Option<TrackRefusal> {
    match properties_to_search(properties, Of::Track) {
        Ok(Ok(searched)) => searched
            .iter()
            .find(|property| property.is_mandatory_track_property())
            .map(|property| TrackRefusal::Unsupported(property.ty())),
        Ok(Err(_)) => Some(TrackRefusal::Malformed),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::coding::parse_exact;
    use crate::data::object_properties_fault;
    use crate::message::decode_track_properties;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len() / 2)
            .map(|at| u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).unwrap())
            .collect()
    }

    /// Column 6 of the table: `type=value`, separated by commas.
    fn searched_text(searched: &[KeyValuePair]) -> String {
        if searched.is_empty() {
            return "none".to_string();
        }
        let pairs: Vec<String> = searched
            .iter()
            .map(|pair| match pair.value() {
                Value::Varint(value) => format!("{:02x}={value}", pair.ty()),
                Value::Bytes(bytes) => {
                    let digits: Vec<String> =
                        bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                    format!("{:02x}={}", pair.ty(), digits.concat())
                }
            })
            .collect();
        pairs.join(",")
    }

    /// What this crate makes of the Properties of an Object.
    fn of_an_object(bytes: &[u8], group_id: u64, object_id: u64) -> (String, String) {
        let properties = parse_exact(bytes, kvp::decode_all).unwrap();
        match object_properties_fault(&properties, group_id, object_id) {
            Ok(()) => {
                let searched = properties_to_search(&properties, Of::Object)
                    .unwrap()
                    .unwrap();
                ("ok".to_string(), searched_text(&searched))
            }
            Err(Error::MalformedTrack(_)) => ("malformed-track".to_string(), "-".to_string()),
            Err(Error::ProtocolViolation(_)) => ("protocol-violation".to_string(), "-".to_string()),
            Err(error) => panic!("{error:?}"),
        }
    }

    /// What this crate makes of the Track Properties at the end of a
    /// `SUBSCRIBE_OK`.
    fn of_a_track(bytes: &[u8]) -> (String, String) {
        let properties = match parse_exact(bytes, decode_track_properties) {
            Ok(properties) => properties,
            Err(Error::ProtocolViolation(_)) => {
                return ("protocol-violation".to_string(), "-".to_string());
            }
            Err(error) => panic!("{error:?}"),
        };
        match track_properties_refusal(&properties) {
            None => {
                let searched = properties_to_search(&properties, Of::Track)
                    .unwrap()
                    .unwrap();
                ("ok".to_string(), searched_text(&searched))
            }
            Some(TrackRefusal::Malformed) => ("malformed-track".to_string(), "-".to_string()),
            Some(TrackRefusal::Unsupported(_)) => {
                ("unsupported-track".to_string(), "-".to_string())
            }
        }
    }

    /// The rows of `fixtures/immutable-properties/`, which `@xmoq/moqt` reads
    /// as well.
    #[test]
    fn immutable_properties_are_searched_or_make_the_track_malformed() {
        let rows = include_str!("../../../fixtures/immutable-properties/properties.tsv");
        let mut read = 0;
        for row in rows.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, of, location, hex, outcome, searched] = fields[..] else {
                panic!("a row of six fields: {row}");
            };
            let bytes = unhex(hex);
            let made = match of {
                "object" => {
                    let (group_id, object_id) = location.split_once('/').unwrap();
                    of_an_object(
                        &bytes,
                        group_id.parse().unwrap(),
                        object_id.parse().unwrap(),
                    )
                }
                "track" => of_a_track(&bytes),
                _ => panic!("{of} in {row}"),
            };
            assert_eq!(
                made,
                (outcome.to_string(), searched.to_string()),
                "{of} {name}"
            );
            read += 1;
        }
        assert_eq!(read, 32);
    }

    /// What a reader above the session searches: column 6 of the rows the
    /// session accepts, and the properties outside for one it refuses.
    #[test]
    fn a_reader_above_the_session_searches_what_the_session_does() {
        let rows = include_str!("../../../fixtures/immutable-properties/properties.tsv");
        let mut read = 0;
        for row in rows.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, of, _, hex, "ok", searched] = fields[..] else {
                continue;
            };
            let of = if of == "object" {
                Of::Object
            } else {
                Of::Track
            };
            let properties = parse_exact(&unhex(hex), kvp::decode_all).unwrap();
            let found = searched_properties(&properties, of);
            assert_eq!(searched_text(&found), searched, "{name}");
            read += 1;
        }
        assert!(read > 0);
        // Immutable Properties inside Immutable Properties: malformed, and
        // nothing is outside.
        let refused = parse_exact(&unhex("0b020b00"), kvp::decode_all).unwrap();
        assert_eq!(
            searched_text(&searched_properties(&refused, Of::Track)),
            "none"
        );
        // Without Immutable Properties the properties are not copied.
        let plain = parse_exact(&unhex("0405"), kvp::decode_all).unwrap();
        assert!(matches!(
            searched_properties(&plain, Of::Object),
            Cow::Borrowed(_)
        ));
    }
}
