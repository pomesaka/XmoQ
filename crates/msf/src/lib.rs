//! The catalog of `draft-ietf-moq-msf-01` §5, read: a JSON document that
//! names the tracks of a broadcast. Section numbers are of that draft.
//!
//! **What is read is what a viewer needs to choose a track**, and far less
//! than `@xmoq/msf` checks: the version, and of every track its name, its
//! packaging, its role, its codec, its size, its bitrate, and the sample
//! rate and the channels of its audio. A document `@xmoq/msf` turns down for another
//! reason (`fixtures/msf-01/catalogs.tsv`) is read here.
//!
//! FIXME: read all of `fixtures/msf-01/catalogs.tsv` and turn down what
//! `@xmoq/msf` turns down. Of it, the rows on the sample rate and the
//! channels are read by the tests.

use serde_json::Value;

/// The one MSF version that is read (§5.1.1), as in `@xmoq/msf`.
pub const MSF_VERSION: &str = "draft-01";

/// The `packaging` of a track whose Objects are LOC (§4.1).
pub const LOC_PACKAGING: &str = "loc";

/// A track of a catalog (§5.2): the fields a viewer chooses by.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogTrack {
    /// §5.2.3.
    pub name: String,
    /// §5.2.2. Left out, the track is in the namespace of the catalog track.
    pub namespace: Option<String>,
    /// §5.2.4.
    pub packaging: String,
    /// §5.2.6.
    pub role: Option<String>,
    /// §5.2.18.
    pub codec: Option<String>,
    /// §5.2.26, in pixels.
    pub width: Option<u64>,
    /// §5.2.27, in pixels.
    pub height: Option<u64>,
    /// §5.2.22: the maximum bitrate, in bits per second.
    pub bitrate: Option<u64>,
    /// §5.2.28: "The number of audio frame samples per second." WHY a
    /// float, where the sizes above are whole numbers: the draft has "JSON
    /// Type: Number" and nothing on whole numbers, `@xmoq/msf` keeps the
    /// number as it is, and both read the same rows of
    /// `fixtures/msf-01/catalogs.tsv`.
    pub samplerate: Option<f64>,
    /// §5.2.29: "A string specifying the audio channel configuration". The
    /// draft's examples and `moqlivemock` have the number of channels in
    /// it, `"2"`; no other form is said.
    pub channel_config: Option<String>,
}

/// Why a document is no catalog that is read here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub String);

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// The tracks of a catalog, in the order the document has them. Fields
/// that are not read are ignored (§5: "A parser MUST ignore fields it does
/// not understand").
///
/// # Errors
///
/// [`Invalid`] for bytes that are no JSON object, another version than
/// [`MSF_VERSION`], no array of tracks, a track without a name or a
/// packaging, and a `samplerate` that is no number or a `channelConfig`
/// that is no string, as in `@xmoq/msf`.
pub fn read_catalog(payload: &[u8]) -> Result<Vec<CatalogTrack>, Invalid> {
    let document: Value =
        serde_json::from_slice(payload).map_err(|error| Invalid(format!("no JSON: {error}")))?;
    let version = document.get("version").and_then(Value::as_str);
    if version != Some(MSF_VERSION) {
        return Err(Invalid(format!(
            "the version is {version:?}, not {MSF_VERSION:?}"
        )));
    }
    let tracks = document
        .get("tracks")
        .and_then(Value::as_array)
        .ok_or_else(|| Invalid("no array of tracks".to_string()))?;
    tracks
        .iter()
        .map(|track| {
            let text = |field: &str| track.get(field).and_then(Value::as_str).map(str::to_string);
            let number = |field: &str| track.get(field).and_then(Value::as_u64);
            // A field of the track that is there has to be of its type.
            let of_type = |field: &str, is: fn(&Value) -> bool, what: &str| match track.get(field) {
                Some(value) if !is(value) => Err(Invalid(format!("{field:?} is no {what}"))),
                _ => Ok(()),
            };
            of_type("samplerate", Value::is_number, "number")?;
            of_type("channelConfig", Value::is_string, "string")?;
            let required = |field: &str| {
                text(field).ok_or_else(|| Invalid(format!("a track without the string {field:?}")))
            };
            Ok(CatalogTrack {
                name: required("name")?,
                namespace: text("namespace"),
                packaging: required("packaging")?,
                role: text("role"),
                codec: text("codec"),
                width: number("width"),
                height: number("height"),
                bitrate: number("bitrate"),
                samplerate: track.get("samplerate").and_then(Value::as_f64),
                channel_config: text("channelConfig"),
            })
        })
        .collect()
}

/// The tracks with the role `video` and LOC packaging whose codec begins
/// with `codec_prefix` (`avc` for H.264: `avc1.…` and `avc3.…`), in the
/// order of the catalog: the versions a viewer with that decoder can
/// choose from.
///
/// WHY not by `altGroup` (§5.2.12), which says which tracks are versions of
/// one content: a catalog of one camera has one such group or none, and a
/// viewer of two cameras would have to choose the camera first, which
/// nothing here does.
pub fn video_tracks<'a>(
    tracks: &'a [CatalogTrack],
    codec_prefix: &'a str,
) -> impl Iterator<Item = &'a CatalogTrack> {
    tracks.iter().filter(move |track| {
        track.role.as_deref() == Some("video")
            && track.packaging == LOC_PACKAGING
            && track
                .codec
                .as_deref()
                .is_some_and(|codec| codec.starts_with(codec_prefix))
    })
}

/// The first of [`video_tracks`].
#[must_use]
pub fn first_video_track<'a>(
    tracks: &'a [CatalogTrack],
    codec_prefix: &'a str,
) -> Option<&'a CatalogTrack> {
    video_tracks(tracks, codec_prefix).next()
}

/// The first track with the role `audio` and LOC packaging whose codec
/// begins with `codec_prefix` (`opus`).
#[must_use]
pub fn first_audio_track<'a>(
    tracks: &'a [CatalogTrack],
    codec_prefix: &str,
) -> Option<&'a CatalogTrack> {
    tracks.iter().find(|track| {
        track.role.as_deref() == Some("audio")
            && track.packaging == LOC_PACKAGING
            && track
                .codec
                .as_deref()
                .is_some_and(|codec| codec.starts_with(codec_prefix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `fixtures/msf-01/catalog-moqlivemock.json`: what `mlmpub` of
    /// moqlivemock v0.16.1 sent, with the values of the README there.
    #[test]
    fn the_catalog_of_moqlivemock_is_read() {
        let payload = include_bytes!("../../../fixtures/msf-01/catalog-moqlivemock.json");
        let tracks = read_catalog(payload).unwrap();
        assert_eq!(tracks.len(), 13);
        let video = first_video_track(&tracks, "avc").unwrap();
        assert_eq!(
            *video,
            CatalogTrack {
                name: "video_400kbps_avc".to_string(),
                namespace: None,
                packaging: "loc".to_string(),
                role: Some("video".to_string()),
                codec: Some("avc3.4D401F".to_string()),
                width: Some(1280),
                height: Some(720),
                bitrate: Some(379_454),
                samplerate: None,
                channel_config: None,
            }
        );
        // The versions a viewer with an H.264 decoder chooses from.
        let versions: Vec<(&str, Option<u64>)> = video_tracks(&tracks, "avc")
            .map(|track| (track.name.as_str(), track.bitrate))
            .collect();
        assert_eq!(
            versions,
            [
                ("video_400kbps_avc", Some(379_454)),
                ("video_600kbps_avc", Some(564_523)),
                ("video_900kbps_avc", Some(848_032)),
            ]
        );
        let opus = first_audio_track(&tracks, "opus").unwrap();
        assert_eq!(
            (
                opus.name.as_str(),
                opus.samplerate,
                opus.channel_config.as_deref()
            ),
            ("audio_monotonic_128kbps_opus", Some(48000.0), Some("2"))
        );
        let aac = first_audio_track(&tracks, "mp4a").unwrap();
        assert_eq!(
            (
                aac.name.as_str(),
                aac.samplerate,
                aac.channel_config.as_deref()
            ),
            ("audio_monotonic_128kbps_aac", Some(48000.0), Some("2"))
        );
        let hevc = first_video_track(&tracks, "hev1").unwrap();
        assert_eq!(hevc.name, "video_400kbps_hevc");
        assert_eq!(first_video_track(&tracks, "vp9"), None);
    }

    /// The rows of `fixtures/msf-01/catalogs.tsv` on the sample rate and
    /// the channels, which `@xmoq/msf` reads as well: what the table says
    /// of each is what is asked of this reader.
    #[test]
    fn the_sample_rate_and_the_channels_are_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/msf-01/catalogs.tsv");
        let rows: Vec<(&str, &str, &str)> = table
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(3, '\t');
                Some((fields.next()?, fields.next()?, fields.next()?))
            })
            .filter(|(name, _, _)| name.contains("samplerate") || name.contains("channel-config"))
            .collect();
        let names: Vec<&str> = rows.iter().map(|(name, _, _)| *name).collect();
        assert_eq!(
            names,
            [
                "audio-track-with-samplerate-and-channel-config",
                "samplerate-as-a-string",
                "samplerate-too-large-for-a-number",
                "samplerate-that-is-no-whole-number",
                "channel-config-as-a-number",
                "channel-config-with-half-a-surrogate-pair",
            ]
        );
        for (name, expected, document) in &rows {
            let read = read_catalog(document.as_bytes());
            assert_eq!(read.is_ok(), *expected == "catalog", "{name}: {read:?}");
        }
        // (the row; its sample rate; its channels)
        let cases = [
            (
                "audio-track-with-samplerate-and-channel-config",
                Some(48000.0),
                Some("1"),
            ),
            ("samplerate-that-is-no-whole-number", Some(44100.5), None),
        ];
        for (row, samplerate, channel_config) in cases {
            let (_, _, document) = rows.iter().find(|(name, _, _)| *name == row).unwrap();
            let tracks = read_catalog(document.as_bytes()).unwrap();
            let audio = first_audio_track(&tracks, "opus").unwrap();
            assert_eq!(
                (audio.samplerate, audio.channel_config.as_deref()),
                (samplerate, channel_config),
                "{row}"
            );
        }
    }

    #[test]
    fn what_is_no_catalog_is_turned_down() {
        // (the document; whether it is read)
        let cases = [
            (r#"{"version":"draft-01","tracks":[]}"#, true),
            (
                r#"{"version":"draft-01","tracks":[{"name":"a","packaging":"loc","x":1}]}"#,
                true,
            ),
            (r#"{"version":"1","tracks":[]}"#, false),
            (r#"{"version":"draft-01"}"#, false),
            (r#"{"version":"draft-01","tracks":[{"name":"a"}]}"#, false),
            (
                r#"{"version":"draft-01","tracks":[{"packaging":"loc"}]}"#,
                false,
            ),
            ("[]", false),
            ("{", false),
        ];
        for (document, read) in cases {
            assert_eq!(
                read_catalog(document.as_bytes()).is_ok(),
                read,
                "{document}"
            );
        }
    }
}
