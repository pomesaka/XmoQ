//! The namespaces and tracks of the transcripts (decided on 2026-10-25): a
//! transcriber that listens to the broadcasts announced under a prefix
//! publishes the track `speech` of `xmoq/transcript/<the prefix's fields>`.
//!
//! WHY a namespace of its own and not a track `transcript` in the
//! broadcast's namespace: that namespace is announced by the broadcast's
//! publisher, and a relay could pass a SUBSCRIBE for any track of it to the
//! publisher rather than to the transcriber; the transcriber would need the
//! publisher's right to publish there; and the track would be missing from
//! the publisher's catalog. Beside `xmoq/chat/<room>`, as the chat's lines
//! are.

use xmoq_moqt::UsageError;
use xmoq_moqt::name::{FullTrackName, TrackNamespace};

/// The fields every namespace of a transcript begins with.
pub const PREFIX: [&str; 2] = ["xmoq", "transcript"];

/// The track of a transcript: what was said, a piece a Group.
pub const SPEECH: &str = "speech";

/// `xmoq/transcript/<the fields of listened>`: the namespace of the
/// transcript of the broadcasts announced under `listened`.
///
/// # Errors
///
/// Fails where a field is empty or the namespace would be longer than
/// §2.4.1 allows: `listened` may have 30 fields at the most.
pub fn transcript_namespace(listened: &TrackNamespace) -> Result<TrackNamespace, UsageError> {
    let fields = PREFIX
        .iter()
        .map(|field| field.as_bytes().to_vec())
        .chain(listened.fields().iter().cloned())
        .collect();
    TrackNamespace::new(fields)
}

/// `xmoq/transcript/<the fields of listened>`, `speech`.
///
/// # Errors
///
/// As [`transcript_namespace`].
pub fn speech_track(listened: &TrackNamespace) -> Result<FullTrackName, UsageError> {
    FullTrackName::new(transcript_namespace(listened)?, SPEECH.as_bytes().to_vec())
}

/// The prefix whose transcript the track is; `None` for any other track.
#[must_use]
pub fn listened_of(track: &FullTrackName) -> Option<TrackNamespace> {
    let [xmoq, transcript, listened @ ..] = track.namespace().fields() else {
        return None;
    };
    let ours = xmoq.as_slice() == PREFIX[0].as_bytes()
        && transcript.as_slice() == PREFIX[1].as_bytes()
        && track.name() == SPEECH.as_bytes();
    if !ours || listened.is_empty() {
        return None;
    }
    TrackNamespace::new(listened.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace(fields: &[&str]) -> TrackNamespace {
        TrackNamespace::new(
            fields
                .iter()
                .map(|field| field.as_bytes().to_vec())
                .collect(),
        )
        .unwrap()
    }

    /// The rows of `fixtures/transcript-tracks/names.tsv`, which
    /// `@xmoq/transcript` reads too: a track, and the prefix it is the
    /// transcript of (`None` for none).
    fn rows() -> Vec<(String, FullTrackName, Option<TrackNamespace>)> {
        let table = include_str!("../../../fixtures/transcript-tracks/names.tsv");
        assert!(table.ends_with('\n') && !table.ends_with("\n\n"));
        table
            .lines()
            .map(|row| {
                let [case, fields, name, listened] = row.split('\t').collect::<Vec<_>>()[..] else {
                    panic!("a row of four columns: {row:?}");
                };
                let of = |json: &str| {
                    let fields: Vec<String> = serde_json::from_str(json).expect("a JSON array");
                    namespace(&fields.iter().map(String::as_str).collect::<Vec<_>>())
                };
                let track = FullTrackName::new(of(fields), name.as_bytes().to_vec()).unwrap();
                let listened = (listened != "none").then(|| of(listened));
                (case.to_owned(), track, listened)
            })
            .collect()
    }

    #[test]
    fn a_track_is_read_and_made_as_the_shared_table_says() {
        let rows = rows();
        assert_eq!(rows.len(), 6);
        for (case, track, listened) in rows {
            assert_eq!(listened_of(&track), listened, "{case}");
            if let Some(listened) = listened {
                assert_eq!(speech_track(&listened).unwrap(), track, "{case}");
            }
        }
    }

    #[test]
    fn a_prefix_of_31_fields_has_no_transcript() {
        let fields: Vec<String> = (0..31).map(|n| n.to_string()).collect();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        assert!(speech_track(&namespace(&fields)).is_err());
        assert!(speech_track(&namespace(&fields[..30])).is_ok());
    }
}
