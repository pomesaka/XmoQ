# xmoq-picture

Pictures of an H.264 video track: decoded to RGB, made smaller by a whole number each way, and written as PNG. `xmoq-watch` decodes with it for the terminal; `Frames` is for a reader that asks for a picture now and then (an agent), which `xmoq-agent` uses. It reads no clock, file or socket: the caller writes the files and removes them.

**Provisional throughout.**

| Module | What it decides |
|---|---|
| `video_decoder` | H.264 to RGB, one NAL unit at a time. The one file that names `openh264` |
| `order` | Which frames the decoder can be given: one that follows the frame before, and the first of a Group that is not older |
| `fit` | The most pixels a picture is let have, and by how much it is made smaller |
| `png_image` | RGB to the bytes of a PNG file. The one file that names `png` |
| `frames` | Which frames are held, when they are decoded, and the name of each picture's file |

## Frames

`take` is given every Object of the video track, in the order they came. **Nothing is decoded unless a picture is asked for**: the frames of the latest Group are held, from its first, and `ask` decodes them and gives the latest picture. WHY, and WHY NOT the other ways (decoding every frame, subscribing when asked): at the top of `src/frames.rs`.

| What | Default | Why |
|---|---|---|
| A picture without being asked for | None (`Settings::every_us`) | A reader that is given pictures it did not ask for reads them all |
| The longest side | 1280 (`longest_side`); a larger picture is made smaller by the least whole number that fits (1920x1080 is 960x540) | The demo's largest picture is 1920x1080 |
| The files kept | 500 (`most_kept`); `Taken::removed` names the oldest beyond them | A PNG of 1280x720 is at most its pixels and a little more, 2.8 MB: 1.4 GB |
| The frames held undecoded | 16 MiB and 600 frames (`MOST_HELD_BYTES`, `MOST_HELD_FRAMES`); beyond them they are decoded at once and let go | A Group of 10 s at 8 Mbit/s is 10 MiB |
| The pixels of a picture | 3840x2160 (`fit::MOST_PIXELS`); a larger one is not given | Four times the largest the demo sends |

All the values are provisional.

A picture that is asked for before the first key frame came is given by the `take` of the first frame that decodes. How long that is waited for is the caller's: `forgo` gives it up. A frame missing from a Group leaves the rest of that Group out (`order`), and the picture is that of the frame before the gap; `Taken::of` says which frame a picture is of.

`begin_again` is for another track or another session: the decoder, the order of the frames and the latest picture are made anew, and the files kept, their numbers and the counts go on, so that `most_kept` bounds the files of every session a reader has and no name is given twice. A publisher in place of another can have smaller Group IDs, whose frames one `Frames` would otherwise leave out as older.

## Dependencies

Read in `Cargo.lock` and the crates' manifests on 2026-10-25.

| Crate | Used for | In | License | Needs to build |
|---|---|---|---|---|
| `openh264` 0.9.8 (`openh264-sys2`) | Decoding H.264 (decided on 2026-10-13, for `xmoq-watch`) | `video_decoder.rs` alone | BSD-2-Clause; it builds Cisco's OpenH264 from the source it carries (BSD-2-Clause) | A C++ compiler (`cc`). `nasm` is used for the assembly where it is found, and left out where it is not |
| `png` 0.18.1 | Writing PNG | `png_image.rs` alone | MIT OR Apache-2.0. What it brings: `flate2` 1.1.10 with `miniz_oxide` (MIT OR Zlib OR Apache-2.0), `fdeflate`, `crc32fast`, `bitflags` (MIT OR Apache-2.0), `adler2` (0BSD OR MIT OR Apache-2.0), `simd-adler32` (MIT) | Nothing but Rust |

## Checks

The tests decode the frames of `fixtures/loc-04/`: Chromium's two chunks (320x180) and `moqlivemock`'s key frame (1280x720). They read the PNG back, and the 8 bars the interop page paints a frame's number as: the key frame shows 0, the next 1.
