//! Pictures of a video track for a reader that asks for one now and then.
//!
//! WHY the frames of the latest Group are held and decoded when a picture
//! is asked for, and not every frame as it comes: a reader asks seldom,
//! and decoding every frame costs a core's share all the time (4 ms a
//! frame of 1080 lines, 30 frames a second, is an eighth of one). A Group
//! begins with a key frame, so its frames are all the decoder needs for its
//! latest picture; the answer then waits for at most one Group's frames to
//! be decoded, and needs no key frame to come. WHY NOT subscribing only
//! when asked: the answer would wait for the subscription and for the next
//! key frame, a Group's length, and the reader has nothing to look at in
//! between.

use std::collections::VecDeque;

use crate::fit;
use crate::order::InOrder;
use crate::png_image::png;
use crate::video_decoder::{Decoded, NoDecoder, VideoDecoder};

/// How large a picture is, how many are kept, and whether one is taken
/// without being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// A picture this often by the Timestamps, without being asked for;
    /// `None`: only when asked for.
    pub every_us: Option<u64>,
    /// The longest side of a picture; a larger one is made smaller.
    pub longest_side: usize,
    /// The files that are kept; the oldest goes for the newest.
    pub most_kept: usize,
}

impl Default for Settings {
    /// **All provisional.** WHY no picture unless asked for: a reader that
    /// is given pictures it did not ask for reads them all. WHY 1280: the
    /// width of 720p; a picture of 1080p is then made 960x540, the least
    /// whole number that fits (`fit`). WHY 500:
    /// a PNG of 1280x720 is no larger than its pixels and a little more,
    /// 2.8 MB, and 500 of them 1.4 GB.
    fn default() -> Self {
        Self {
            every_us: None,
            longest_side: 1280,
            most_kept: 500,
        }
    }
}

/// The most bytes of frames that are held undecoded: **provisional**.
/// Beyond them the frames held are decoded at once and let go, and the
/// latest picture is kept in their place.
///
/// WHY 16 MiB: a Group of a second at 2.5 Mbit/s (a 720p video) is
/// 300 KiB, and a Group of 10 s at 8 Mbit/s 10 MiB.
pub const MOST_HELD_BYTES: usize = 16 << 20;

/// The most frames that are held undecoded, for the same reason as
/// [`MOST_HELD_BYTES`]: 10 s at 60 frames a second. **Provisional.**
pub const MOST_HELD_FRAMES: usize = 600;

/// An Object of the video track.
#[derive(Debug, Clone, Copy)]
pub struct VideoObject<'a> {
    /// Its Group ID.
    pub group: u64,
    /// Its Object ID.
    pub object: u64,
    /// Its Timestamp of LOC, in microseconds.
    pub timestamp_us: u64,
    /// The frame as a byte stream with start codes.
    pub annex_b: &'a [u8],
}

/// Which Object a picture is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Of {
    /// Its Group ID.
    pub group: u64,
    /// Its Object ID.
    pub object: u64,
    /// Its Timestamp of LOC, in microseconds.
    pub timestamp_us: u64,
}

/// A picture to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// The name of its file, which no picture of this [`Frames`] had
    /// before; names sort in the order they were taken.
    pub file_name: String,
    /// The bytes of the file.
    pub png: Vec<u8>,
    /// The picture's size in pixels, after it was made smaller.
    pub width: u32,
    /// See [`Taken::width`].
    pub height: u32,
    /// The frame it is of: the latest that was decoded, which can be older
    /// than the latest that came where frames after it did not decode.
    pub of: Of,
    /// The files to be removed for it: those beyond [`Settings::most_kept`].
    pub removed: Vec<String>,
}

/// What was done with the Objects that were given.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Every Object [`Frames::take`] was given.
    pub given: u64,
    /// Before the first key frame, and the rest of a Group after one that
    /// did not come ([`InOrder`]).
    pub left_out: u64,
    /// Decoded to a picture.
    pub decoded: u64,
    /// Given to the decoder, which gave no picture.
    pub no_picture: u64,
    /// More pixels than are handled ([`fit::MOST_PIXELS`]).
    pub too_large: u64,
    /// Held frames that were let go undecoded: those of a Group that a
    /// newer one began before they were asked for, and those held at
    /// [`Frames::begin_again`].
    pub passed_over: u64,
    /// Pictures that were taken.
    pub taken: u64,
    /// Taken, and not written as PNG.
    pub not_written: u64,
}

/// A frame that is held for the decoder.
struct Held {
    of: Of,
    annex_b: Vec<u8>,
}

/// The pictures of one video track.
pub struct Frames {
    settings: Settings,
    decoder: VideoDecoder,
    order: InOrder,
    /// The frames of the latest Group that have not been decoded, from its
    /// first or from after the last that was.
    held: VecDeque<Held>,
    held_bytes: usize,
    /// The latest picture, in `rgb`, and the frame it is of.
    picture: Option<((usize, usize), Of)>,
    rgb: Vec<u8>,
    shrunk: Vec<u8>,
    /// A picture was asked for and not given yet.
    asked: bool,
    /// The Timestamp of the last picture taken.
    last_taken_us: Option<u64>,
    kept: VecDeque<String>,
    counts: Counts,
}

impl Frames {
    /// # Errors
    ///
    /// [`NoDecoder`] where the decoder cannot be made.
    pub fn new(settings: Settings) -> Result<Self, NoDecoder> {
        Ok(Self {
            settings,
            decoder: VideoDecoder::new()?,
            order: InOrder::default(),
            held: VecDeque::new(),
            held_bytes: 0,
            picture: None,
            rgb: Vec::new(),
            shrunk: Vec::new(),
            asked: false,
            last_taken_us: None,
            kept: VecDeque::new(),
            counts: Counts::default(),
        })
    }

    /// Begins again with another track, or the same one on another session:
    /// the decoder, the order of the frames, the frames held and the latest
    /// picture are made anew. The files kept, their numbers, the counts and
    /// a picture that was asked for go on.
    ///
    /// WHY not a new [`Frames`]: [`Settings::most_kept`] bounds the files
    /// of every track a reader is given, and the names of a new one would
    /// be those of the files that are there. WHY the order anew: a
    /// publisher in place of another can have smaller Group IDs, whose
    /// frames would be left out as older.
    ///
    /// # Errors
    ///
    /// [`NoDecoder`] where the decoder cannot be made; nothing is changed
    /// then.
    pub fn begin_again(&mut self) -> Result<(), NoDecoder> {
        self.decoder = VideoDecoder::new()?;
        self.order = InOrder::default();
        self.counts.passed_over += u64::try_from(self.held.len()).unwrap_or(u64::MAX);
        self.held.clear();
        self.held_bytes = 0;
        self.picture = None;
        self.last_taken_us = None;
        Ok(())
    }

    /// What was done so far, since [`Frames::new`].
    #[must_use]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Whether a picture was asked for and not given yet.
    #[must_use]
    pub fn is_asked(&self) -> bool {
        self.asked
    }

    /// Asks for a picture: the latest the frames that came give, at once
    /// where there is one, else that of the first frame that decodes after
    /// this ([`Frames::take`] gives it). How long that is waited for is the
    /// caller's to say: [`Frames::forgo`] gives it up.
    pub fn ask(&mut self) -> Option<Taken> {
        self.asked = true;
        self.answer()
    }

    /// Gives up a picture that was asked for and not given.
    pub fn forgo(&mut self) {
        self.asked = false;
    }

    /// Takes one Object of the track, every one in the order it came. Says
    /// the picture to write where one was asked for and can now be given,
    /// or is due by [`Settings::every_us`].
    pub fn take(&mut self, object: VideoObject<'_>) -> Option<Taken> {
        self.counts.given += 1;
        if !self.order.take(object.group, object.object) {
            self.counts.left_out += 1;
            return None;
        }
        if object.object == 0 {
            self.counts.passed_over += u64::try_from(self.held.len()).unwrap_or(u64::MAX);
            self.held.clear();
            self.held_bytes = 0;
        }
        self.held.push_back(Held {
            of: Of {
                group: object.group,
                object: object.object,
                timestamp_us: object.timestamp_us,
            },
            annex_b: object.annex_b.to_vec(),
        });
        self.held_bytes = self.held_bytes.saturating_add(object.annex_b.len());
        if self.held_bytes > MOST_HELD_BYTES || self.held.len() > MOST_HELD_FRAMES {
            self.decode_held();
        }
        let due = self.settings.every_us.is_some_and(|every| {
            self.last_taken_us
                .is_none_or(|last| object.timestamp_us.abs_diff(last) >= every)
        });
        if due {
            self.asked = true;
        }
        if self.asked { self.answer() } else { None }
    }

    /// Decodes the frames that are held, and lets them go.
    fn decode_held(&mut self) {
        while let Some(frame) = self.held.pop_front() {
            match self.decoder.decode(&frame.annex_b, &mut self.rgb) {
                Decoded::Picture(size) => {
                    self.counts.decoded += 1;
                    self.picture = Some((size, frame.of));
                }
                Decoded::TooLarge(_) => {
                    self.counts.too_large += 1;
                    self.picture = None;
                }
                Decoded::Nothing => self.counts.no_picture += 1,
            }
        }
        self.held_bytes = 0;
    }

    /// The latest picture, where one was asked for and there is one.
    fn answer(&mut self) -> Option<Taken> {
        self.decode_held();
        let (full, of) = self.picture?;
        self.asked = false;
        // WHY the Timestamp of the frame, and not of when it was asked
        // for: `every_us` counts from the picture before.
        self.last_taken_us = Some(of.timestamp_us);
        self.counts.taken += 1;
        let longest = self.settings.longest_side;
        let by = fit::by(full, (longest, longest));
        let (rgb, size) = if by > 1 {
            let size = fit::shrink(&self.rgb, full, by, &mut self.shrunk);
            (self.shrunk.as_slice(), size)
        } else {
            (self.rgb.as_slice(), full)
        };
        let (Ok(width), Ok(height), Ok(file)) =
            (u32::try_from(size.0), u32::try_from(size.1), png(rgb, size))
        else {
            self.counts.not_written += 1;
            return None;
        };
        // WHY the count first: the same Group and Object can come again (a
        // publisher that begins anew), or be asked for twice, and a name
        // taken twice would have the second file removed for the first.
        let file_name = format!("{:06}-g{}-o{}.png", self.counts.taken, of.group, of.object);
        self.kept.push_back(file_name.clone());
        let beyond = self
            .kept
            .len()
            .saturating_sub(self.settings.most_kept.max(1));
        Some(Taken {
            file_name,
            png: file,
            width,
            height,
            of,
            removed: self.kept.drain(..beyond).collect(),
        })
    }
}
