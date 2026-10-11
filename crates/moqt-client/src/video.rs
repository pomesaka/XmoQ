//! `--annex-b-out`: the Objects of a video track of LOC, written as an
//! H.264 byte stream (Annex B) that begins with a key frame.

use std::fs::File;
use std::io::Write;

use anyhow::{Context, Result};
use xmoq_loc::draft_04 as loc;
use xmoq_moqt::data::{ObjectStatus, SubgroupHeader, SubgroupObject};
use xmoq_moqt::kvp::Value;

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
    file: File,
    path: String,
    /// The Group that is written, and the Object ID that comes next in it.
    at: Option<(u64, u64)>,
    frames: u64,
    groups: u64,
    skipped: u64,
    bytes: usize,
}

impl AnnexBWriter {
    /// The writer for `--annex-b-out`, where the flag was given.
    pub fn for_flag(path: Option<&str>) -> Result<Option<Self>> {
        path.map(Self::create).transpose()
    }

    fn create(path: &str) -> Result<Self> {
        Ok(Self {
            file: File::create(path).with_context(|| format!("--annex-b-out {path}"))?,
            path: path.to_string(),
            at: None,
            frames: 0,
            groups: 0,
            skipped: 0,
            bytes: 0,
        })
    }

    /// Takes one Object, and prints `VIDEO_FRAME` for a frame it wrote.
    pub fn take(&mut self, group_id: u64, object: &SubgroupObject) -> Result<()> {
        if object.status != ObjectStatus::Normal {
            return Ok(());
        }
        let frame = loc::to_annex_b(&object.payload)
            .with_context(|| format!("the payload of {group_id}/{}", object.object_id))?;
        let types = loc::h264_nal_unit_types(&frame);
        let begins_a_group = object.object_id == 0
            && types.contains(&IDR_SLICE)
            && self.at.is_none_or(|(group, _)| group_id > group);
        let comes_next = self.at == Some((group_id, object.object_id));
        if !begins_a_group && !comes_next {
            self.skipped += 1;
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
        self.file
            .write_all(&frame)
            .with_context(|| format!("writing {}", self.path))?;
        self.at = Some((group_id, object.object_id + 1));
        self.frames += 1;
        self.groups += u64::from(begins_a_group);
        self.bytes += frame.len();
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
        println!(
            "VIDEO_WRITTEN file={} frames={} groups={} skipped={} bytes={}",
            self.path, self.frames, self.groups, self.skipped, self.bytes
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
    if let Some(video) = video {
        return video.take(header.group_id, object);
    }
    crate::report::object(header, object);
    Ok(())
}
