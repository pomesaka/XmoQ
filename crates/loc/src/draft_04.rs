//! `draft-ietf-moq-loc-04`. Section numbers are of that revision.

/// §2.3.1.1, ID 0x10: "Timestamp of the encoded media frame encoded as
/// vi64."
const TIMESTAMP: u64 = 0x10;
/// §2.3.1.2, ID 0x08: "The number of Timestamp units per second, encoded as
/// vi64."
const TIMESCALE: u64 = 0x08;

/// §2.1.4: "The start code value is 1 in network byte order".
const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// When a frame is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// The Timestamp.
    pub timestamp: u64,
    /// The Timescale of the Object, where it has one. Without one the
    /// Timestamp is wall-clock time in microseconds.
    pub timescale: Option<u64>,
}

/// Why Object Properties or a payload are not LOC's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub String);

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// Reads the Timestamp and the Timescale out of the Object Properties of an
/// Object, given as their types and, for the ones that carry an integer,
/// their values. `None` for an Object without a Timestamp.
///
/// What carries them is the caller's: `MoQ` Transport, for one.
///
/// Invalid, as in `@xmoq/loc`, where the draft says nothing of a receiver:
/// a Timestamp or a Timescale that comes twice or is no integer, a
/// Timescale of 0, and a Timescale without a Timestamp.
///
/// # Errors
///
/// [`Invalid`] with what is wrong.
pub fn read_timing(
    properties: impl IntoIterator<Item = (u64, Option<u64>)>,
) -> Result<Option<Timing>, Invalid> {
    let (mut timestamp, mut timescale) = (None, None);
    for (ty, value) in properties {
        let (name, slot) = match ty {
            TIMESTAMP => ("Timestamp", &mut timestamp),
            TIMESCALE => ("Timescale", &mut timescale),
            _ => continue,
        };
        let Some(value) = value else {
            return Err(Invalid(format!("the {name} is no integer")));
        };
        if slot.replace(value).is_some() {
            return Err(Invalid(format!("the {name} comes twice")));
        }
    }
    if timescale == Some(0) {
        return Err(Invalid("the Timescale is 0".to_string()));
    }
    match timestamp {
        Some(timestamp) => Ok(Some(Timing {
            timestamp,
            timescale,
        })),
        None if timescale.is_some() => Err(Invalid("a Timescale without a Timestamp".to_string())),
        None => Ok(None),
    }
}

/// The Timestamp of an Object as wall-clock microseconds (§2.3.1.1:
/// without a Timescale): what the terminal viewer plays by and the
/// transcriber cuts by. `None` for an Object without a Timestamp, with
/// properties that are invalid ([`read_timing`]), or with a Timescale of its
/// own, which neither handles: the demo's publishers send none.
#[must_use]
pub fn wall_clock_timestamp(
    properties: impl IntoIterator<Item = (u64, Option<u64>)>,
) -> Option<u64> {
    let timing = read_timing(properties).ok()??;
    timing.timescale.is_none().then_some(timing.timestamp)
}

/// Makes a start code of every length prefix of a video payload (§2.1.3,
/// §2.1.4). A payload that has start codes already comes out with the same
/// bytes: "A length value of 1 SHOULD be interpreted as a start code rather
/// than a length" (§2.1.3), and what follows one is left alone.
///
/// # Errors
///
/// [`Invalid`] for a length of 0, a length that goes beyond the payload,
/// and fewer than four bytes where a length is to begin.
pub fn to_annex_b(payload: &[u8]) -> Result<Vec<u8>, Invalid> {
    let mut out = payload.to_vec();
    let mut at = 0usize;
    while at < payload.len() {
        let left = payload.len() - at;
        let Some(prefix) = payload.get(at..at + 4) else {
            return Err(Invalid(format!(
                "{left} bytes at {at}, where a length of 4 bytes is to begin"
            )));
        };
        let length = prefix
            .iter()
            .fold(0usize, |n, byte| (n << 8) | usize::from(*byte));
        if length == 1 {
            break;
        }
        if length == 0 {
            return Err(Invalid(format!("a NAL unit of no bytes at {at}")));
        }
        if length > left - 4 {
            return Err(Invalid(format!(
                "a NAL unit of {length} bytes at {at}, where {} are left",
                left - 4
            )));
        }
        if let Some(slot) = out.get_mut(at..at + 4) {
            slot.copy_from_slice(&START_CODE);
        }
        at += 4 + length;
    }
    Ok(out)
}

/// The NAL unit type (the low 5 bits of its first byte) of every NAL unit
/// of a payload in the form [`to_annex_b`] gives, for H.264.
#[must_use]
pub fn h264_nal_unit_types(annex_b: &[u8]) -> Vec<u8> {
    annex_b
        .windows(5)
        .filter(|window| window.starts_with(&START_CODE))
        .filter_map(|window| window.last().map(|first| first & 0x1f))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    type Properties = &'static [(u64, Option<u64>)];

    fn fixture(text: &str) -> Vec<u8> {
        let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
        digits
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    /// `fixtures/loc-04/`: two payloads `mlmpub` of moqlivemock v0.16.1
    /// sent, with the values of the README there.
    #[test]
    fn the_video_payloads_of_moqlivemock_become_annex_b() {
        let key = fixture(include_str!(
            "../../../fixtures/loc-04/video-key-length-prefixed-from-moqlivemock.hex"
        ));
        let delta = fixture(include_str!(
            "../../../fixtures/loc-04/video-delta-length-prefixed-from-moqlivemock.hex"
        ));
        assert_eq!((key.len(), delta.len()), (21_533, 931));
        let key_out = to_annex_b(&key).unwrap();
        let delta_out = to_annex_b(&delta).unwrap();
        assert_eq!((key_out.len(), delta_out.len()), (21_533, 931));
        // A sequence parameter set (7), a picture parameter set (8) and a
        // slice of an IDR picture (5); a slice of another picture (1).
        assert_eq!(h264_nal_unit_types(&key_out), [7, 8, 5]);
        assert_eq!(h264_nal_unit_types(&delta_out), [1]);
        // Once it has start codes it stays as it is.
        assert_eq!(to_annex_b(&key_out).unwrap(), key_out);
    }

    #[test]
    fn a_payload_that_is_cut_is_invalid() {
        // (the payload; whether it is read)
        let cases: [(&[u8], bool); 6] = [
            (&[], true),
            (&[0, 0, 0, 2, 0x65, 0x01], true),
            (&[0, 0, 0, 1, 0x65], true),
            (&[0, 0, 0, 3, 0x65, 0x01], false),
            (&[0, 0, 0, 0], false),
            (&[0, 0, 0, 1, 0x65, 0, 0], true),
        ];
        for (payload, read) in cases {
            assert_eq!(to_annex_b(payload).is_ok(), read, "{payload:?}");
        }
        assert!(to_annex_b(&[0, 0, 0, 1, 0x41, 9, 9, 0, 0]).is_ok());
        assert!(to_annex_b(&[0, 0, 0, 2, 0x41, 9, 0, 0]).is_err());
    }

    #[test]
    fn the_timestamp_of_an_object_is_read_from_its_properties() {
        // The first Object of the fixture: type 0x10, 1791690527000000.
        let timing = |timestamp, timescale| {
            Some(Timing {
                timestamp,
                timescale,
            })
        };
        // (the Object Properties; what is read, `Err` as `None`)
        let cases: [(Properties, Option<Option<Timing>>); 7] = [
            (
                &[(0x10, Some(1_791_690_527_000_000))],
                Some(timing(1_791_690_527_000_000, None)),
            ),
            (
                &[(0x08, Some(90_000)), (0x10, Some(3))],
                Some(timing(3, Some(90_000))),
            ),
            (&[], Some(None)),
            // A property that is not LOC's is passed over.
            (&[(0x3e, Some(1))], Some(None)),
            (&[(0x10, Some(1)), (0x10, Some(2))], None),
            (&[(0x08, Some(0)), (0x10, Some(2))], None),
            (&[(0x08, Some(1))], None),
        ];
        for (properties, read) in cases {
            assert_eq!(
                read_timing(properties.iter().copied()).ok(),
                read,
                "{properties:?}"
            );
        }
    }
}
