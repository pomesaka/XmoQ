//! `--annex-b-out`: the Objects of a video track of LOC, written as an
//! H.264 byte stream (Annex B) that begins with a key frame.

use std::fs::File;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::{
    ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject, SubgroupStreamEncoder,
};
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt_io::{Connection, SendStream};

/// The frames of an H.264 byte stream, as Groups that each begin with a key
/// frame: a frame is the NAL units up to and with a slice.
pub type Groups = Vec<Vec<Vec<u8>>>;

/// The flags about a video track.
#[derive(Debug, Default)]
pub struct VideoFlags {
    /// `--annex-b-out`: where `--mode subscribe` writes the frames of a LOC
    /// video track, as an H.264 byte stream.
    pub annex_b_out: Option<String>,
    /// `--viewers N`: `--mode subscribe` runs N sessions at once, each
    /// with a subscription of its own, and each says how late the frames
    /// of the LOC video track came (`VIDEO_STATS`) in place of the lines of
    /// its Objects. 0 without the flag. An experiment's: for measuring a
    /// relay with many subscribers of one track.
    pub viewers: usize,
    /// `--video-in <file>`: a publisher sends, for every `send`, the next
    /// Group of this H.264 byte stream on one stream, a frame every 40
    /// milliseconds, each with the wall-clock time as its Timestamp of LOC;
    /// after the last Group comes the first again. An experiment's as well.
    pub video_in: Option<Groups>,
    /// `--video-streams object`: with `--video-in`, every frame goes on a
    /// stream of its own, as the one Object of a Subgroup with its Object
    /// ID; `group`, which is what is done without the flag, puts a Group
    /// on one stream. An experiment's: a frame on its own stream does not
    /// wait for one before it that was lost on the way.
    pub stream_per_object: bool,
}

impl VideoFlags {
    /// Reads a flag that takes a value. Whether it was one of these.
    pub fn read(&mut self, flag: &str, value: &str) -> Result<bool> {
        match flag {
            "--annex-b-out" => self.annex_b_out = Some(value.to_string()),
            "--viewers" => self.viewers = value.parse().context("--viewers")?,
            "--video-in" => {
                let bytes = std::fs::read(value).with_context(|| format!("--video-in {value}"))?;
                let groups = groups_of(&bytes);
                anyhow::ensure!(!groups.is_empty(), "--video-in {value}: no key frame");
                self.video_in = Some(groups);
            }
            "--video-streams" => {
                self.stream_per_object = match value {
                    "object" => true,
                    "group" => false,
                    other => anyhow::bail!("--video-streams {other}: group or object"),
                };
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// Cuts a byte stream with start codes of 4 bytes into frames, and the
/// frames into Groups at every key frame. What comes before the first key
/// frame is left out.
fn groups_of(annex_b: &[u8]) -> Groups {
    let starts: Vec<usize> = annex_b
        .windows(4)
        .enumerate()
        .filter(|(_, window)| *window == [0, 0, 0, 1])
        .map(|(at, _)| at)
        .collect();
    let mut groups: Groups = Vec::new();
    let mut frame: Vec<u8> = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(annex_b.len());
        let Some(nal) = annex_b.get(*start..end) else {
            continue;
        };
        frame.extend_from_slice(nal);
        let ty = nal.get(4).map_or(0, |first| first & 0x1f);
        // A slice ends the frame it is of: 5 of an IDR picture, 1 of another.
        if ty == IDR_SLICE {
            groups.push(vec![std::mem::take(&mut frame)]);
        } else if ty == 1 {
            let whole = std::mem::take(&mut frame);
            if let Some(group) = groups.last_mut() {
                group.push(whole);
            }
        }
    }
    groups
}

/// `--video-in`: sends one Group on one stream, at the pace of 25 frames a
/// second.
pub async fn send_group<C: Connection>(
    connection: &C,
    track_alias: u64,
    group_id: u64,
    flags: &VideoFlags,
) -> Result<()> {
    let groups = flags.video_in.as_deref().unwrap_or(&[]);
    let index = usize::try_from(group_id).unwrap_or(0) % groups.len().max(1);
    let frames = groups.get(index).map_or(&[][..], Vec::as_slice);
    if flags.stream_per_object {
        return send_objects(connection, track_alias, group_id, frames).await;
    }
    let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
        track_alias,
        group_id,
        subgroup_id: SubgroupId::Zero,
        publisher_priority: Some(128),
        properties: true,
        end_of_group: true,
        first_object: true,
    });
    let mut data = connection.open_uni().await?;
    let mut bytes = 0;
    let mut next = tokio::time::Instant::now();
    for (object_id, frame) in (0u64..).zip(frames) {
        tokio::time::sleep_until(next).await;
        next += std::time::Duration::from_millis(40);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let now = u64::try_from(now.as_micros()).unwrap_or(u64::MAX);
        encoder.object(&SubgroupObject {
            object_id,
            // The Timestamp of LOC-04 (0x10), without a Timescale.
            properties: vec![KeyValuePair::varint(0x10, now)?],
            status: ObjectStatus::Normal,
            payload: frame.clone(),
        })?;
        bytes += frame.len();
        if let Err(error) = data.write_all(&encoder.take_bytes()).await {
            anyhow::bail!("could not write the subgroup stream: {error:?}");
        }
    }
    data.finish();
    println!(
        "SENT group={group_id} objects={} payload_len={bytes}",
        frames.len()
    );
    Ok(())
}

/// `--video-streams object`: the frames of one Group, each on a stream of
/// its own, at the pace of 25 frames a second.
async fn send_objects<C: Connection>(
    connection: &C,
    track_alias: u64,
    group_id: u64,
    frames: &[Vec<u8>],
) -> Result<()> {
    let mut bytes = 0;
    let mut next = tokio::time::Instant::now();
    for (object_id, frame) in (0u64..).zip(frames) {
        tokio::time::sleep_until(next).await;
        next += std::time::Duration::from_millis(40);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let now = u64::try_from(now.as_micros()).unwrap_or(u64::MAX);
        // §11.4.2: each stream is a Subgroup of its own, with the Object ID
        // as its Subgroup ID.
        let mut encoder = SubgroupStreamEncoder::new(&SubgroupHeader {
            track_alias,
            group_id,
            subgroup_id: SubgroupId::Explicit(object_id),
            publisher_priority: Some(128),
            properties: true,
            end_of_group: false,
            first_object: true,
        });
        encoder.object(&SubgroupObject {
            object_id,
            properties: vec![KeyValuePair::varint(0x10, now)?],
            status: ObjectStatus::Normal,
            payload: frame.clone(),
        })?;
        bytes += frame.len();
        let mut data = connection.open_uni().await?;
        if let Err(error) = data.write_all(&encoder.take_bytes()).await {
            anyhow::bail!("could not write the subgroup stream: {error:?}");
        }
        data.finish();
    }
    println!(
        "SENT group={group_id} objects={} payload_len={bytes}",
        frames.len()
    );
    Ok(())
}

/// The NAL unit type of a slice of an IDR picture, in H.264.
const IDR_SLICE: u8 = 5;

/// Takes the Objects of a track as they come and writes the frames that
/// can be decoded in the order they came in.
///
/// It begins with Object 0 of a Group whose payload has an IDR picture, and
/// from then on takes the next Object ID of that Group, or Object 0 of a
/// later Group. WHY nothing is held back for an Object that is behind: a
/// frame that depends on one that did not come cannot be decoded, and the
/// next Group begins anew. Whatever else comes is counted as skipped.
pub struct AnnexBWriter {
    /// Where the frames go; with `--viewers` nowhere, and how late each
    /// came is kept instead.
    file: Option<File>,
    path: String,
    /// `--viewers`: for every frame, the microseconds from its Timestamp
    /// to the clock of this machine when it came. A Timestamp of LOC
    /// without a Timescale is wall-clock time.
    late: Vec<u64>,
    /// `--viewers`: every Object that came, also one that was not taken
    /// for being out of order (`Self::note_arrival`).
    arrivals: Vec<(u64, u64, u64)>,
    /// The Group that is written, and the Object ID that comes next in it.
    at: Option<(u64, u64)>,
    frames: u64,
    groups: u64,
    skipped: u64,
    /// Objects that came before the first key frame: a subscriber that
    /// comes in the middle of a Group has to wait for the next.
    before: u64,
    bytes: usize,
}

impl AnnexBWriter {
    /// The writer for `--annex-b-out`, where the flag was given.
    pub fn for_flag(path: Option<&str>) -> Result<Option<Self>> {
        path.map(Self::create).transpose()
    }

    /// The writer of `--viewers`: it writes nothing, and counts.
    pub fn counting() -> Self {
        Self {
            file: None,
            path: "none".to_string(),
            late: Vec::new(),
            arrivals: Vec::new(),
            at: None,
            frames: 0,
            groups: 0,
            skipped: 0,
            before: 0,
            bytes: 0,
        }
    }

    fn create(path: &str) -> Result<Self> {
        Ok(Self {
            file: Some(File::create(path).with_context(|| format!("--annex-b-out {path}"))?),
            path: path.to_string(),
            late: Vec::new(),
            arrivals: Vec::new(),
            at: None,
            frames: 0,
            groups: 0,
            skipped: 0,
            before: 0,
            bytes: 0,
        })
    }

    /// `--viewers`: when an Object came, whatever came before it. In the
    /// order they came, as (Group, Object ID, microseconds from its
    /// Timestamp to this machine's clock).
    fn note_arrival(&mut self, group_id: u64, object: &SubgroupObject) {
        let timestamp = object
            .properties
            .iter()
            .find_map(|pair| match pair.value() {
                // The Timestamp of LOC-04.
                Value::Varint(value) if pair.ty() == 0x10 => Some(*value),
                _ => None,
            });
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let now = u64::try_from(now.as_micros()).unwrap_or(u64::MAX);
        if let Some(timestamp) = timestamp {
            self.arrivals
                .push((group_id, object.object_id, now.saturating_sub(timestamp)));
        }
    }

    /// Takes one Object, and prints `VIDEO_FRAME` for a frame it wrote.
    pub fn take(&mut self, group_id: u64, object: &SubgroupObject) -> Result<()> {
        if object.status != ObjectStatus::Normal {
            return Ok(());
        }
        let frame = loc::to_annex_b(&object.payload)
            .with_context(|| format!("the payload of {group_id}/{}", object.object_id))?;
        let types = loc::h264_nal_unit_types(&frame);
        if self.file.is_none() {
            self.note_arrival(group_id, object);
        }
        let begins_a_group = object.object_id == 0
            && types.contains(&IDR_SLICE)
            && self.at.is_none_or(|(group, _)| group_id > group);
        let comes_next = self.at == Some((group_id, object.object_id));
        if !begins_a_group && !comes_next {
            if self.at.is_none() {
                self.before += 1;
            } else {
                self.skipped += 1;
            }
            return Ok(());
        }
        let timing = loc::read_timing(object.properties.iter().map(|pair| {
            let value = match pair.value() {
                Value::Varint(value) => Some(*value),
                Value::Bytes(_) => None,
            };
            (pair.ty(), value)
        }))
        .with_context(|| format!("the properties of {group_id}/{}", object.object_id))?;
        if let Some(file) = &mut self.file {
            file.write_all(&frame)
                .with_context(|| format!("writing {}", self.path))?;
        }
        self.at = Some((group_id, object.object_id + 1));
        self.frames += 1;
        self.groups += u64::from(begins_a_group);
        self.bytes += frame.len();
        if self.file.is_none() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let now = u64::try_from(now.as_micros()).unwrap_or(u64::MAX);
            if let Some(timing) = timing {
                self.late.push(now.saturating_sub(timing.timestamp));
            }
            return Ok(());
        }
        let nal: Vec<String> = types.iter().map(u8::to_string).collect();
        println!(
            "VIDEO_FRAME group={group_id} object={} timestamp={} nal={} bytes={}",
            object.object_id,
            timing.map_or("none".to_string(), |timing| timing.timestamp.to_string()),
            nal.join(","),
            frame.len(),
        );
        Ok(())
    }
}

/// WHY here: the client stops reading in several ways (the number of
/// Objects it was told, its time limit, an error), and a driver gets the
/// line for each.
impl Drop for AnnexBWriter {
    fn drop(&mut self) {
        if self.file.is_none() {
            self.late.sort_unstable();
            // The value below which `percent` of the frames were.
            let at = |percent: usize| {
                let index =
                    (self.late.len() * percent / 100).min(self.late.len().saturating_sub(1));
                self.late.get(index).copied().unwrap_or(0)
            };
            println!(
                "VIDEO_STATS frames={} groups={} before_the_first_key_frame={} skipped={} bytes={} late_us_p50={} late_us_p99={} late_us_max={}",
                self.frames,
                self.groups,
                self.before,
                self.skipped,
                self.bytes,
                at(50),
                at(99),
                self.late.last().copied().unwrap_or(0),
            );
            // WHY one line: a run with hundreds of viewers has one for each.
            let arrivals: Vec<String> = self
                .arrivals
                .iter()
                .map(|(group, object, late)| format!("{group}:{object}:{late}"))
                .collect();
            println!("VIDEO_ARRIVALS {}", arrivals.join(","));
            return;
        }
        println!(
            "VIDEO_WRITTEN file={} frames={} groups={} skipped={} bytes={}",
            self.path,
            self.frames,
            self.groups,
            self.skipped + self.before,
            self.bytes
        );
    }
}

/// An Object of the subscription: a frame for the writer where there is
/// one, and an `OBJECT` line otherwise. WHY not both: a frame of video is
/// tens of kilobytes, which `OBJECT` prints as hex.
pub fn take_or_print(
    video: &mut Option<AnnexBWriter>,
    header: &SubgroupHeader,
    object: &SubgroupObject,
) -> Result<()> {
    // With `--catalog-video`, the Object of a catalog track.
    crate::catalog::offer(&object.payload);
    if let Some(video) = video {
        return video.take(header.group_id, object);
    }
    crate::report::object(header, object);
    Ok(())
}
