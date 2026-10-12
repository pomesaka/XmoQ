//! Which tracks of a catalog the viewer plays.

use xmoq_msf::{CatalogTrack, first_audio_track, first_video_track};

/// The tracks that are played.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chosen<'a> {
    /// The picture.
    pub video: &'a CatalogTrack,
    /// The sound.
    pub audio: &'a CatalogTrack,
}

/// What a catalog lacks for the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// No H.264 video track with LOC packaging.
    Video,
    /// No Opus audio track with LOC packaging.
    Audio,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Video => "the catalog has no H.264 video track with LOC packaging",
            Self::Audio => "the catalog has no Opus audio track with LOC packaging",
        })
    }
}

impl std::error::Error for Missing {}

/// The first H.264 video track and the first Opus audio track of a catalog,
/// both with LOC packaging: what the viewer has decoders for.
///
/// WHY the first, where a catalog can have several: the demo's publisher
/// puts its main version first and a small one after it, and nothing here
/// measures the path to choose by.
///
/// # Errors
///
/// [`Missing`] for a catalog without one of the two.
pub fn choose(tracks: &[CatalogTrack]) -> Result<Chosen<'_>, Missing> {
    Ok(Chosen {
        video: first_video_track(tracks, "avc").ok_or(Missing::Video)?,
        audio: first_audio_track(tracks, "opus").ok_or(Missing::Audio)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(document: &str) -> Result<(String, String), Missing> {
        let tracks = xmoq_msf::read_catalog(document.as_bytes()).unwrap();
        choose(&tracks).map(|chosen| (chosen.video.name.clone(), chosen.audio.name.clone()))
    }

    #[test]
    fn the_first_tracks_the_viewer_can_decode_are_chosen() {
        let track = |name: &str, role: &str, codec: &str, packaging: &str| {
            format!(
                r#"{{"name":"{name}","packaging":"{packaging}","isLive":true,"role":"{role}","codec":"{codec}"}}"#
            )
        };
        let catalog = |tracks: &[String]| {
            format!(
                r#"{{"version":"draft-01","tracks":[{}]}}"#,
                tracks.join(",")
            )
        };
        let both = |video: &str, audio: &str| Ok((video.to_string(), audio.to_string()));
        // (the tracks of the catalog; what is chosen)
        let cases = [
            (
                vec![
                    track("video", "video", "avc3.42E01F", "loc"),
                    track("video-low", "video", "avc3.42E01F", "loc"),
                    track("audio", "audio", "opus", "loc"),
                ],
                both("video", "audio"),
            ),
            // Codecs the viewer has no decoder for are passed over.
            (
                vec![
                    track("v-hevc", "video", "hev1.1.6.L93.B0", "loc"),
                    track("a-aac", "audio", "mp4a.40.2", "loc"),
                    track("a-opus", "audio", "opus", "loc"),
                    track("v-avc", "video", "avc1.42E01F", "loc"),
                ],
                both("v-avc", "a-opus"),
            ),
            // A track that is not LOC is passed over.
            (
                vec![
                    track("v-cmaf", "video", "avc1.42E01F", "cmaf"),
                    track("v", "video", "avc1.42E01F", "loc"),
                    track("a", "audio", "opus", "loc"),
                ],
                both("v", "a"),
            ),
            (
                vec![track("a", "audio", "opus", "loc")],
                Err(Missing::Video),
            ),
            (
                vec![track("v", "video", "avc3.42E01F", "loc")],
                Err(Missing::Audio),
            ),
            (vec![], Err(Missing::Video)),
        ];
        for (tracks, chosen) in cases {
            assert_eq!(names(&catalog(&tracks)), chosen, "{tracks:?}");
        }
    }
}
