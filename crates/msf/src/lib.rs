//! The catalog of `draft-ietf-moq-msf-01` §5, read: a JSON document that
//! names the tracks of a broadcast. Section numbers are of that draft.
//!
//! **What is kept is what a viewer needs to choose a track**, and less than
//! `@xmoq/msf` keeps: the version, and of every track its name, its
//! packaging, its role, its codec, its size, its bitrate, and the sample
//! rate and the channels of its audio. What is turned down is what
//! `@xmoq/msf` turns down: both read every row of
//! `fixtures/msf-01/catalogs.tsv`.

use serde_json::Value;

/// The one MSF version that is read (§5.1.1), as in `@xmoq/msf`.
pub const MSF_VERSION: &str = "draft-01";

/// The name of the catalog track of a broadcast (§5), as in `@xmoq/msf`.
pub const CATALOG_TRACK: &str = "catalog";

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

/// Why a payload gives no tracks to read, told apart as `CatalogRead` of
/// `@xmoq/msf` tells them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoCatalog {
    /// A delta update (§5.1.6, §5.3), which changes an earlier catalog of
    /// its Group. Told apart and not applied, as in `@xmoq/msf`.
    DeltaUpdate,
    /// A catalog of another MSF version (§5.1.1: "A subscriber MUST NOT
    /// attempt to parse a catalog version which it does not understand").
    UnsupportedVersion(String),
    /// No catalog of this version: what is wrong, for a log.
    Invalid(String),
}

impl std::fmt::Display for NoCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeltaUpdate => f.write_str("a delta update, which is not applied"),
            Self::UnsupportedVersion(version) => {
                write!(f, "the version is {version:?}, not {MSF_VERSION:?}")
            }
            Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for NoCatalog {}

/// The most bytes of a payload [`read_catalog`] reads, as
/// `DEFAULT_MAX_CATALOG_LENGTH` of `@xmoq/msf`, whose WHY this shares: the
/// payload is parsed whole, and an Object of `MoQ` Transport can be far
/// larger than any catalog. Provisional.
pub const MAX_CATALOG_LENGTH: usize = 1024 * 1024;

fn invalid(reason: impl Into<String>) -> NoCatalog {
    NoCatalog::Invalid(reason.into())
}

/// Whether a string is of nothing but characters other than `%` and
/// variable references (§5.4.1: "Variables are denoted by enclosing the
/// variable name in percent characters (%). Variable names MUST consist of
/// alphanumeric characters, hyphens, and underscores", and "The percent
/// character (%) MUST NOT appear in catalog field values except as part of a
/// variable reference"). ASCII alphanumeric, and a name of at least one
/// character, as `OF_VARIABLES` of `@xmoq/msf` has it.
fn of_variables(value: &str) -> bool {
    let mut rest = value;
    while let Some(open) = rest.find('%') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('%') else {
            return false;
        };
        let name = &after[..close];
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return false;
        }
        rest = &after[close + 1..];
    }
    true
}

/// The first string value of the document that is not [`of_variables`].
/// Every value is looked at, those of fields that are not read among them;
/// keys are no field values.
fn string_with_percent(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => (!of_variables(text)).then_some(text.as_str()),
        Value::Array(values) => values.iter().find_map(string_with_percent),
        Value::Object(fields) => fields.values().find_map(string_with_percent),
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

/// The tracks of a catalog, in the order the document has them, by the
/// rules `@xmoq/msf` reads it by: both read every row of
/// `fixtures/msf-01/catalogs.tsv` alike. Fields that are not read are
/// ignored (§5: "A parser MUST ignore fields it does not understand"), and
/// `null` is a value of another type in every field, never an absent one.
///
/// WHY `serde_json` as it is: it refuses half of a surrogate pair and a number
/// too large for a double anywhere in the document, and `@xmoq/msf` refuses
/// the same (`readCatalog`, where the WHY is). A key that comes twice is
/// read as the later one, as `JSON.parse` reads it (`serde_json` 1.0.152,
/// tried on 2026-10-25).
///
/// # Errors
///
/// [`NoCatalog`] for a payload that is no catalog of [`MSF_VERSION`]: a
/// delta update, another version, or one that is invalid.
pub fn read_catalog(payload: &[u8]) -> Result<Vec<CatalogTrack>, NoCatalog> {
    if payload.len() > MAX_CATALOG_LENGTH {
        return Err(invalid(format!(
            "the payload has {} bytes, more than the {MAX_CATALOG_LENGTH} that are read",
            payload.len()
        )));
    }
    // RFC 1952 §2.3.1: a GZIP member begins with 1f 8b, as in `@xmoq/msf`.
    if payload.starts_with(&[0x1f, 0x8b]) {
        return Err(invalid(
            "the payload begins like a GZIP stream; compressed catalogs are not read",
        ));
    }
    let document: Value =
        serde_json::from_slice(payload).map_err(|error| invalid(format!("no JSON: {error}")))?;
    let Value::Object(document) = &document else {
        return Err(invalid("the catalog is not a JSON object"));
    };
    // Before anything else of the document, as in `@xmoq/msf`.
    if let Some(value) = document.values().find_map(string_with_percent) {
        return Err(invalid(format!(
            "{value:?} has a \"%\" outside a variable reference (§5.4.1)"
        )));
    }
    // §5.1.6, §5.3: "A delta update MUST include the Delta Update ... field
    // with at least one operation. It MUST NOT contain an instance of a
    // Tracks ... field or an MSF version ... field." Before the version,
    // which a delta update does not have.
    if let Some(delta) = document.get("deltaUpdate") {
        return Err(match delta.as_array() {
            None => invalid("\"deltaUpdate\" is not an array"),
            Some(operations) if operations.is_empty() => {
                invalid("\"deltaUpdate\" has no operation")
            }
            Some(_) if document.contains_key("tracks") || document.contains_key("version") => {
                invalid("a delta update has \"tracks\" or \"version\"")
            }
            Some(_) => NoCatalog::DeltaUpdate,
        });
    }
    let catalog = Fields(document, "the catalog");
    let version = catalog.required("version", Value::as_str, "string")?;
    if version != MSF_VERSION {
        return Err(NoCatalog::UnsupportedVersion(version.to_string()));
    }
    catalog.optional("generatedAt", Value::as_f64, "number")?;
    // §5.1.3: "This field MUST NOT be included if it is FALSE."
    if catalog.optional("isComplete", Value::as_bool, "boolean")? == Some(false) {
        return Err(invalid("\"isComplete\" is false"));
    }
    let tracks = catalog.required("tracks", Value::as_array, "array")?;
    let mut names = std::collections::HashSet::new();
    let mut read = Vec::with_capacity(tracks.len());
    for (index, track) in tracks.iter().enumerate() {
        let track = read_track(track, &format!("track {index}"))?;
        // §5.2.3: "Within the catalog, track names MUST be unique per
        // namespace." A track without a namespace is in the catalog's own,
        // which is not known here, as in `@xmoq/msf`.
        if !names.insert((track.namespace.clone(), track.name.clone())) {
            return Err(invalid(format!(
                "track {index}: the name {:?} comes twice in its namespace",
                track.name
            )));
        }
        read.push(track);
    }
    Ok(read)
}

/// The fields of a JSON object, and what to call it in a reason.
struct Fields<'a>(&'a serde_json::Map<String, Value>, &'a str);

impl<'a> Fields<'a> {
    /// A field that is of the JSON type the draft gives it, or absent.
    fn optional<T>(
        &self,
        key: &str,
        as_type: fn(&'a Value) -> Option<T>,
        what: &str,
    ) -> Result<Option<T>, NoCatalog> {
        let Fields(fields, whose) = self;
        match fields.get(key) {
            None => Ok(None),
            Some(value) => as_type(value)
                .map(Some)
                .ok_or_else(|| invalid(format!("{whose}: {key:?} is not a {what}"))),
        }
    }

    /// A field with "Required: Yes", or one a MUST asks for here.
    fn required<T>(
        &self,
        key: &str,
        as_type: fn(&'a Value) -> Option<T>,
        what: &str,
    ) -> Result<T, NoCatalog> {
        self.optional(key, as_type, what)?
            .ok_or_else(|| invalid(format!("{}: {key:?} is missing", self.1)))
    }
}

/// A track of the catalog, with the fields `checkTrack` of `@xmoq/msf`
/// checks and the WHY it has: with LOC packaging a codec and a bitrate are
/// asked for (§5.2.18, §5.2.22), and every other field is optional and of
/// its JSON type.
fn read_track(value: &Value, whose: &str) -> Result<CatalogTrack, NoCatalog> {
    let Value::Object(fields) = value else {
        return Err(invalid(format!("{whose} is not an object")));
    };
    let track = Fields(fields, whose);
    let text = |key| track.optional(key, Value::as_str, "string");
    let number = |key| track.optional(key, Value::as_f64, "number");
    let name = track.required("name", Value::as_str, "string")?;
    let packaging = track.required("packaging", Value::as_str, "string")?;
    track.required("isLive", Value::as_bool, "boolean")?;
    let media = packaging == LOC_PACKAGING;
    let codec = text("codec")?;
    let bitrate = number("bitrate")?;
    if media && codec.is_none() {
        return Err(invalid(format!("{whose}: \"codec\" is missing")));
    }
    if media && bitrate.is_none() {
        return Err(invalid(format!("{whose}: \"bitrate\" is missing")));
    }
    for key in ["renderGroup", "altGroup", "framerate"] {
        number(key)?;
    }
    text("encryptionScheme")?;
    // WHY whole numbers for the sizes and the bitrate, where `@xmoq/msf`
    // keeps any number: a viewer chooses by them, and one that is not whole
    // or is below 0 is kept from it as absent; the document is read alike.
    let integer = |key| track.optional(key, Value::as_u64, "number").ok().flatten();
    Ok(CatalogTrack {
        name: name.to_string(),
        namespace: text("namespace")?.map(str::to_string),
        packaging: packaging.to_string(),
        role: text("role")?.map(str::to_string),
        codec: codec.map(str::to_string),
        width: number("width")?.and(integer("width")),
        height: number("height")?.and(integer("height")),
        bitrate: bitrate.and(integer("bitrate")),
        samplerate: number("samplerate")?,
        channel_config: text("channelConfig")?.map(str::to_string),
    })
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

    /// Every row of `fixtures/msf-01/catalogs.tsv`, which `@xmoq/msf` reads
    /// as well: what each document is read as.
    #[test]
    fn every_document_is_read_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/msf-01/catalogs.tsv");
        let mut read = 0;
        for line in table.lines() {
            let mut fields = line.splitn(3, '\t');
            let (Some(name), Some(expected), Some(document)) =
                (fields.next(), fields.next(), fields.next())
            else {
                panic!("a row of three fields: {line}");
            };
            let outcome = match read_catalog(document.as_bytes()) {
                Ok(_) => "catalog",
                Err(NoCatalog::DeltaUpdate) => "delta-update",
                Err(NoCatalog::UnsupportedVersion(_)) => "unsupported-version",
                Err(NoCatalog::Invalid(_)) => "invalid",
            };
            assert_eq!(outcome, expected, "{name}");
            read += 1;
        }
        assert_eq!(read, 80);
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
}
