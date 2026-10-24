//! [`Frames`] with the frames of real encoders (`fixtures/loc-04/`).

use crate::frames::{Counts, Frames, Of, Settings, VideoObject};

fn fixture(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

/// The first two chunks of Chromium's encoder: 320x180, all black.
fn chromium() -> (Vec<u8>, Vec<u8>) {
    (
        fixture(include_str!(
            "../../../fixtures/loc-04/video-key-annex-b-from-chromium.hex"
        )),
        fixture(include_str!(
            "../../../fixtures/loc-04/video-delta-annex-b-from-chromium.hex"
        )),
    )
}

/// The size and the pixels of a PNG file.
fn read(file: &[u8]) -> ((u32, u32), Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(file))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    ((info.width, info.height), pixels)
}

/// The 8 bars the interop page paints a frame's number as (`paintNumber`
/// in `interop/page.ts`, the highest bit at the left), read at the middle
/// of each on the row `y`: 1 for white, 0 for black, 2 for neither.
fn bars(rgb: &[u8], width: usize, y: usize) -> Vec<u8> {
    (0..8)
        .map(|bar| {
            let x = width / 8 * bar + width / 16;
            let red = rgb.get((y * width + x) * 3).copied().unwrap_or_default();
            match red {
                0..64 => 0,
                192.. => 1,
                _ => 2,
            }
        })
        .collect()
}

fn object(group: u64, object: u64, timestamp_us: u64, annex_b: &[u8]) -> VideoObject<'_> {
    VideoObject {
        group,
        object,
        timestamp_us,
        annex_b,
    }
}

#[test]
fn a_picture_is_taken_only_when_asked_for() {
    let (key, delta) = chromium();
    let mut frames = Frames::new(Settings::default()).unwrap();
    assert_eq!(frames.take(object(7, 0, 5_000_000, &key)), None);
    assert_eq!(frames.take(object(7, 1, 5_040_000, &delta)), None);
    let taken = frames.ask().unwrap();
    assert_eq!(
        (
            taken.file_name.as_str(),
            taken.width,
            taken.height,
            taken.of
        ),
        (
            "000001-g7-o1.png",
            320,
            180,
            Of {
                group: 7,
                object: 1,
                timestamp_us: 5_040_000
            }
        )
    );
    assert!(!frames.is_asked());
    let (size, pixels) = read(&taken.png);
    assert_eq!(size, (320, 180));
    assert_eq!(bars(&pixels, 320, 90), [0, 0, 0, 0, 0, 0, 0, 1]);
    // Asked again, the same picture is another file.
    assert_eq!(frames.ask().unwrap().file_name, "000002-g7-o1.png");
}

#[test]
fn a_picture_asked_for_before_a_key_frame_is_given_with_the_first() {
    let (key, delta) = chromium();
    let mut frames = Frames::new(Settings::default()).unwrap();
    assert_eq!(frames.ask(), None);
    assert!(frames.is_asked());
    assert_eq!(frames.take(object(3, 1, 0, &delta)), None);
    let taken = frames.take(object(4, 0, 40_000, &key)).unwrap();
    assert_eq!(taken.file_name, "000001-g4-o0.png");
    assert_eq!(bars(&read(&taken.png).1, 320, 90), [0; 8]);
    assert!(!frames.is_asked());
    assert_eq!(frames.take(object(4, 1, 80_000, &delta)), None);
}

#[test]
fn a_picture_that_is_forgone_is_not_given() {
    let (key, _) = chromium();
    let mut frames = Frames::new(Settings::default()).unwrap();
    assert_eq!(frames.ask(), None);
    frames.forgo();
    assert!(!frames.is_asked());
    assert_eq!(frames.take(object(4, 0, 0, &key)), None);
}

#[test]
fn a_key_frame_of_moqlivemock_is_a_png_of_1280x720() {
    let key = xmoq_loc::draft_04::to_annex_b(&fixture(include_str!(
        "../../../fixtures/loc-04/video-key-length-prefixed-from-moqlivemock.hex"
    )))
    .unwrap();
    let mut frames = Frames::new(Settings::default()).unwrap();
    frames.take(object(1_791_690_527, 0, 1_791_690_527_000_000, &key));
    let taken = frames.ask().unwrap();
    assert_eq!((taken.width, taken.height), (1280, 720));
    assert_eq!(read(&taken.png).0, (1280, 720));
}

#[test]
fn a_picture_larger_than_the_longest_side_is_made_smaller() {
    let (key, _) = chromium();
    // (the longest side; the size of the picture)
    let cases = [
        (1280, (320, 180)),
        (320, (320, 180)),
        (319, (160, 90)),
        (100, (80, 45)),
    ];
    for (longest_side, expected) in cases {
        let mut frames = Frames::new(Settings {
            longest_side,
            ..Settings::default()
        })
        .unwrap();
        frames.take(object(0, 0, 0, &key));
        let taken = frames.ask().unwrap();
        assert_eq!((taken.width, taken.height), expected, "{longest_side}");
        assert_eq!(read(&taken.png).0, expected, "{longest_side}");
    }
}

#[test]
fn with_every_a_picture_is_taken_that_often_by_the_timestamps() {
    type Case = (&'static [(u64, u64, u64)], &'static [Option<&'static str>]);
    let (key, delta) = chromium();
    // (Group, Object, Timestamp in ms; the name of the file taken), every 5 s
    let cases: [Case; 4] = [
        (
            &[(1, 0, 0), (1, 1, 40), (1, 2, 4_999), (1, 3, 5_000)],
            &[
                Some("000001-g1-o0.png"),
                None,
                None,
                Some("000002-g1-o3.png"),
            ],
        ),
        // A Group's first is not taken for being one.
        (
            &[(1, 0, 0), (2, 0, 2_000), (3, 0, 5_000)],
            &[Some("000001-g1-o0.png"), None, Some("000002-g3-o0.png")],
        ),
        // Before the first key frame nothing is decoded, and nothing taken.
        (
            &[(1, 1, 0), (1, 2, 6_000), (2, 0, 6_040)],
            &[None, None, Some("000001-g2-o0.png")],
        ),
        // A Timestamp that goes back: a publisher that began again.
        (
            &[(9, 0, 60_000), (10, 0, 1_000), (11, 0, 3_000)],
            &[Some("000001-g9-o0.png"), Some("000002-g10-o0.png"), None],
        ),
    ];
    for (objects, expected) in cases {
        let mut frames = Frames::new(Settings {
            every_us: Some(5_000_000),
            ..Settings::default()
        })
        .unwrap();
        let taken: Vec<Option<String>> = objects
            .iter()
            .map(|(group, number, ms)| {
                let frame = if *number == 0 { &key } else { &delta };
                frames
                    .take(object(*group, *number, ms * 1000, frame))
                    .map(|taken| taken.file_name)
            })
            .collect();
        let expected: Vec<Option<String>> =
            expected.iter().map(|name| name.map(String::from)).collect();
        assert_eq!(taken, expected, "{objects:?}");
    }
}

#[test]
fn the_oldest_files_are_removed_beyond_the_most_kept() {
    let (key, _) = chromium();
    let mut frames = Frames::new(Settings {
        most_kept: 2,
        ..Settings::default()
    })
    .unwrap();
    frames.take(object(0, 0, 0, &key));
    let removed: Vec<Vec<String>> = (0..4).map(|_| frames.ask().unwrap().removed).collect();
    assert_eq!(
        removed,
        [
            vec![],
            vec![],
            vec!["000001-g0-o0.png".to_string()],
            vec!["000002-g0-o0.png".to_string()],
        ]
    );
}

#[test]
fn frames_are_decoded_only_when_a_picture_is_asked_for() {
    let (key, delta) = chromium();
    let mut frames = Frames::new(Settings::default()).unwrap();
    for (group, number, frame) in [
        (1, 1, delta.as_slice()),
        (1, 0, &key),
        (1, 1, &delta),
        (1, 3, &delta),
        (2, 0, &key),
        (2, 1, &delta),
    ] {
        frames.take(object(group, number, 0, frame));
    }
    assert_eq!(
        frames.counts(),
        Counts {
            given: 6,
            left_out: 2,
            passed_over: 2,
            ..Counts::default()
        }
    );
    frames.ask().unwrap();
    frames.take(object(2, 2, 0, &[0, 0, 0, 1, 0x09, 0xf0]));
    frames.ask().unwrap();
    assert_eq!(
        frames.counts(),
        Counts {
            given: 7,
            left_out: 2,
            decoded: 2,
            no_picture: 1,
            too_large: 0,
            passed_over: 2,
            taken: 2,
            not_written: 0,
        }
    );
}

#[test]
fn frames_beyond_what_is_held_are_decoded_without_being_asked_for() {
    let (key, delta) = chromium();
    let mut frames = Frames::new(Settings::default()).unwrap();
    frames.take(object(1, 0, 0, &key));
    for number in 1..=600 {
        frames.take(object(1, number, 0, &delta));
    }
    // 601 frames are one more than are held.
    assert_eq!(frames.counts().decoded + frames.counts().no_picture, 601);
    frames.take(object(1, 601, 0, &delta));
    assert_eq!(frames.counts().decoded + frames.counts().no_picture, 601);
}
