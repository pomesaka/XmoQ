//! Which tracks of a catalog the viewer plays.

use xmoq_msf::{CatalogTrack, first_audio_track, video_tracks};

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
    /// No such track has the name that was asked for.
    VideoNamed,
    /// No Opus audio track with LOC packaging.
    Audio,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Video => "the catalog has no H.264 video track with LOC packaging",
            Self::VideoNamed => {
                "the catalog has no H.264 video track with LOC packaging and the name asked for"
            }
            Self::Audio => "the catalog has no Opus audio track with LOC packaging",
        })
    }
}

impl std::error::Error for Missing {}

/// Which version of the video is played, where a catalog has several
/// (`--video-version`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VideoVersion {
    /// The first of the catalog.
    #[default]
    First,
    /// The one with this name.
    Named(String),
    /// The tallest one that is no taller than so many pixels.
    HeightAtMost(u64),
    /// The one with the most bits a second that has no more than so many.
    BitrateAtMost(u64),
}

/// The H.264 video track that is asked for and the first Opus audio track
/// of a catalog, both with LOC packaging: what the viewer has decoders
/// for.
///
/// WHY the first where nothing is asked, though a catalog can have
/// several: the demo's publisher puts its main version first and a small
/// one after it, and nothing here measures the path to choose by.
///
/// With a limit, the versions are measured by what the catalog says of
/// them (`height`, `bitrate`), and **where none is within the limit the
/// smallest by that measure is taken**: a viewer that asks for at most 500
/// kbit/s of a broadcast whose smallest version has 600 is better off with
/// that one than with no picture. A version the catalog does not say the
/// measure of is not counted; where it says it of none, the first is
/// taken. Of two that are alike the earlier in the catalog is taken.
///
/// # Errors
///
/// [`Missing`] for a catalog without one of the two, or without a video
/// track of the name that was asked for.
pub fn choose<'a>(
    tracks: &'a [CatalogTrack],
    version: &VideoVersion,
) -> Result<Chosen<'a>, Missing> {
    let versions: Vec<&CatalogTrack> = video_tracks(tracks, "avc").collect();
    let first = *versions.first().ok_or(Missing::Video)?;
    let within = |measure: fn(&CatalogTrack) -> Option<u64>, most: u64| {
        let measured = versions
            .iter()
            .filter_map(|track| measure(track).map(|size| (size, *track)));
        // `min_by_key` and `max_by_key` differ in which of two alike they
        // take, so the order is turned for the latter: the earlier wins.
        let fits = measured
            .clone()
            .filter(|(size, _)| *size <= most)
            .rev()
            .max_by_key(|(size, _)| *size);
        fits.or_else(|| measured.min_by_key(|(size, _)| *size))
            .map_or(first, |(_, track)| track)
    };
    let video = match version {
        VideoVersion::First => first,
        VideoVersion::Named(name) => versions
            .iter()
            .find(|track| track.name == *name)
            .copied()
            .ok_or(Missing::VideoNamed)?,
        VideoVersion::HeightAtMost(most) => within(|track| track.height, *most),
        VideoVersion::BitrateAtMost(most) => within(|track| track.bitrate, *most),
    };
    Ok(Chosen {
        video,
        audio: first_audio_track(tracks, "opus").ok_or(Missing::Audio)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(document: &str) -> Result<(String, String), Missing> {
        let tracks = xmoq_msf::read_catalog(document.as_bytes()).unwrap();
        choose(&tracks, &VideoVersion::First)
            .map(|chosen| (chosen.video.name.clone(), chosen.audio.name.clone()))
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

    /// The versions of the demo's publisher with `?ladder=1`, and of
    /// `moqlivemock` (`fixtures/msf-01/catalog-moqlivemock.json`: three of
    /// 1280x720 with 379,454, 564,523 and 848,032 bits a second).
    #[test]
    fn the_version_that_is_asked_for_is_chosen() {
        let demo = r#"{"version":"draft-01","tracks":[
            {"name":"video","packaging":"loc","role":"video","codec":"avc3.42E01F","width":640,"height":360,"bitrate":1500000},
            {"name":"video-low","packaging":"loc","role":"video","codec":"avc3.42E01F","width":320,"height":180,"bitrate":500000},
            {"name":"video-bare","packaging":"loc","role":"video","codec":"avc3.42E01F"},
            {"name":"audio","packaging":"loc","role":"audio","codec":"opus"}]}"#;
        let bare = r#"{"version":"draft-01","tracks":[
            {"name":"a","packaging":"loc","role":"video","codec":"avc3.42E01F"},
            {"name":"b","packaging":"loc","role":"video","codec":"avc3.42E01F"},
            {"name":"audio","packaging":"loc","role":"audio","codec":"opus"}]}"#;
        let mock = include_str!("../../../fixtures/msf-01/catalog-moqlivemock.json");
        let named = |name: &str| VideoVersion::Named(name.to_string());
        // (the catalog; what is asked for; the video track that is chosen)
        let cases = [
            (demo, VideoVersion::First, Ok("video")),
            (demo, named("video-low"), Ok("video-low")),
            (demo, named("audio"), Err(Missing::VideoNamed)),
            (demo, VideoVersion::HeightAtMost(360), Ok("video")),
            (demo, VideoVersion::HeightAtMost(359), Ok("video-low")),
            (demo, VideoVersion::HeightAtMost(1080), Ok("video")),
            // None is within the limit: the smallest.
            (demo, VideoVersion::HeightAtMost(100), Ok("video-low")),
            (
                demo,
                VideoVersion::BitrateAtMost(1_000_000),
                Ok("video-low"),
            ),
            (demo, VideoVersion::BitrateAtMost(1_500_000), Ok("video")),
            (demo, VideoVersion::BitrateAtMost(1), Ok("video-low")),
            // The catalog says the measure of none: the first.
            (bare, VideoVersion::HeightAtMost(360), Ok("a")),
            (bare, VideoVersion::BitrateAtMost(1), Ok("a")),
            (
                mock,
                VideoVersion::BitrateAtMost(600_000),
                Ok("video_600kbps_avc"),
            ),
            (
                mock,
                VideoVersion::BitrateAtMost(564_522),
                Ok("video_400kbps_avc"),
            ),
            (
                mock,
                VideoVersion::BitrateAtMost(2_000_000),
                Ok("video_900kbps_avc"),
            ),
            // Three of one height: the earliest of them.
            (
                mock,
                VideoVersion::HeightAtMost(720),
                Ok("video_400kbps_avc"),
            ),
        ];
        for (document, version, expected) in cases {
            let tracks = xmoq_msf::read_catalog(document.as_bytes()).unwrap();
            let chosen = choose(&tracks, &version).map(|chosen| chosen.video.name.as_str());
            assert_eq!(chosen, expected, "{version:?}");
        }
    }
}
