//! The catalog of `draft-ietf-moq-msf-01` §5, read: a JSON document that
//! names the tracks of a broadcast. Section numbers are of that draft.
//!
//! **What is read is what a viewer needs to choose a track**, and far less
//! than `@xmoq/msf` checks: the version, and of every track its name, its
//! packaging, its role, its codec and its size. A document `@xmoq/msf`
//! turns down for another reason (`fixtures/msf-01/catalogs.tsv`) is read
//! here.
//!
//! FIXME: read `fixtures/msf-01/catalogs.tsv` and turn down what
//! `@xmoq/msf` turns down.

use serde_json::Value;

/// The one MSF version that is read (§5.1.1), as in `@xmoq/msf`.
pub const MSF_VERSION: &str = "draft-01";

/// The `packaging` of a track whose Objects are LOC (§4.1).
pub const LOC_PACKAGING: &str = "loc";

/// A track of a catalog (§5.2): the fields a viewer chooses by.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// [`MSF_VERSION`], no array of tracks, and a track without a name or a
/// packaging.
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
            })
        })
        .collect()
}

/// The first track with the role `video` and LOC packaging whose codec
/// begins with `codec_prefix` (`avc` for H.264: `avc1.…` and `avc3.…`).
#[must_use]
pub fn first_video_track<'a>(
    tracks: &'a [CatalogTrack],
    codec_prefix: &str,
) -> Option<&'a CatalogTrack> {
    tracks.iter().find(|track| {
        track.role.as_deref() == Some("video")
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
            }
        );
        let hevc = first_video_track(&tracks, "hev1").unwrap();
        assert_eq!(hevc.name, "video_400kbps_hevc");
        assert_eq!(first_video_track(&tracks, "vp9"), None);
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
