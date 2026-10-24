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

    #[test]
    fn the_speech_track_of_a_prefix_and_back() {
        let listened = namespace(&["xmoq", "demo"]);
        let track = speech_track(&listened).unwrap();
        assert_eq!(
            track.namespace(),
            &namespace(&["xmoq", "transcript", "xmoq", "demo"])
        );
        assert_eq!(track.name(), b"speech");
        assert_eq!(listened_of(&track), Some(listened));
    }

    #[test]
    fn other_tracks_are_no_transcript() {
        let cases = [
            (vec!["xmoq", "transcript", "xmoq", "demo"], "lines"),
            (vec!["xmoq", "chat", "r"], "speech"),
            // A transcript of nothing: no prefix has no field.
            (vec!["xmoq", "transcript"], "speech"),
        ];
        for (fields, name) in cases {
            let track = FullTrackName::new(namespace(&fields), name.as_bytes().to_vec()).unwrap();
            assert_eq!(listened_of(&track), None, "{fields:?} {name}");
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
