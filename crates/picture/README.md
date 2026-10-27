# xmoq-picture

Pictures of an H.264 video track: decoded to RGB, made smaller by a whole number each way, and written as PNG. `xmoq-watch` decodes with it for the terminal; `Frames` is for a reader that asks for a picture now and then (an agent), which `xmoq-agent` uses. It reads no clock, file or socket: the caller writes the files and removes them. **Nothing is decoded unless a picture is asked for**: why, at the top of `src/frames.rs`.

**Provisional throughout.** `openh264` is named by `src/video_decoder.rs` alone, and `png` by `src/png_image.rs` alone.

## Dependencies

| Crate | Used for | In | License | Needs to build |
|---|---|---|---|---|
| `openh264` 0.9.8 (`openh264-sys2`) | Decoding H.264 | `video_decoder.rs` alone | BSD-2-Clause; it builds Cisco's OpenH264 from the source it carries (BSD-2-Clause) | A C++ compiler (`cc`). `nasm` is used for the assembly where it is found, and left out where it is not |
| `png` 0.18.1 | Writing PNG | `png_image.rs` alone | MIT OR Apache-2.0. What it brings: `flate2` 1.1.10 with `miniz_oxide` (MIT OR Zlib OR Apache-2.0), `fdeflate`, `crc32fast`, `bitflags` (MIT OR Apache-2.0), `adler2` (0BSD OR MIT OR Apache-2.0), `simd-adler32` (MIT) | Nothing but Rust |
