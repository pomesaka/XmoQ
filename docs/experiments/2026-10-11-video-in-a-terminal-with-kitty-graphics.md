# Video in a terminal with the Kitty graphics protocol, and two ways to decode it

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, video decoding and Kitty Graphics experiment

## Objective

To have what is needed to choose a decoder for the CLI, which is undecided ([open questions](../open-questions.md)), and to see whether frames can be drawn in a terminal fast enough. **Nothing here decides the decoder**: the two that were tried are the ones with the fewest steps to try, and both are in `spikes/`.

## Environment

macOS, loopback. ffmpeg 8.1 (a binary of mise's installs, not on the path). The crate `openh264` 0.9.8, which builds Cisco's OpenH264 from source with a C++ compiler. The Rust client and the own relay of the working copy this record is part of; `mlmpub` of moqlivemock v0.16.1; Chromium 156 for the page. **No terminal: every run wrote the escape codes to a file or to `/dev/null`.**

## Commands

```sh
# What is drawn: spikes/kitty-graphics (no dependencies). A pattern, or rgb24 frames from stdin.
kitty-frames --pattern 1280x720 --frames 100 --fps 1000 --medium direct|file > /dev/null

# Decoding 1: ffmpeg as a process. The client writes Annex B into a FIFO.
spikes/kitty-graphics/play-mlmpub.sh 75        # mlmpub, 1280x720, no relay
spikes/kitty-graphics/play-page.sh 100         # a page that encodes 320x180, through the own relay

# Decoding 2: the openh264 crate, on files the client wrote (spikes/h264-decoders).
h264-decoders mlmpub.h264 ; h264-decoders page.h264
h264-decoders mlmpub.h264 --rgb | kitty-frames --raw 1280x720 > /dev/null
```

`kitty-frames` sends every frame as the image with the id 1 again (`a=T,f=24,i=1,p=1,q=2,C=1`), at the place the cursor was saved at; `--medium direct` has the pixels in the escape codes, base64 in chunks of 4096 bytes, and `--medium file` writes them to a temporary file for the terminal to read and delete (`t=t`).

## Observed results

Making the escape codes, without a terminal to draw them, 100 frames each:

| Size | Medium | Bytes a frame to the terminal | Milliseconds a frame to make and write |
|---|---|---|---|
| 320x180 | direct | 230,952 | 0.21 |
| 320x180 | file | 167 | 0.09 |
| 1280x720 | direct | 3,694,540 | 3.68 |
| 1280x720 | file | 168 | 1.37 |

Decoding:

- **ffmpeg as a process**: 75 of 75 frames of `mlmpub` and 100 of 100 of the page's video came out as RGB and were handed to `kitty-frames`, in 4.0 and 4.3 seconds, for video of 3 and 4 seconds: about a second passes before the first frame. With `-fflags nobuffer`, which was tried first for that, 25 of 60 frames of a file did not come out; the scripts do not use it.
- **The `openh264` crate**: 60 of 60 frames of each file, no error. `mlmpub`'s video is Main profile (`avc3.4D401F`), 1280x720: 1.43 ms a frame to decode, 2.55 ms with the conversion to RGB. The page's is Constrained Baseline, 320x180: 0.03 ms a frame. The README of OpenH264 lists Constrained Baseline alone among what its decoder has; the Main profile stream of `mlmpub` decoded all the same.

## Limitations and blockers

- **Nothing was seen on a screen.** Whether Ghostty draws the frames, how many a second, with how much of a processor, and whether sending the image anew under one id flickers, takes a person at a terminal: `spikes/kitty-graphics/play-page.sh` from the repository root, in Ghostty, after `mise run interop` has run once. `MEDIUM=file` in front of it tries the temporary file. The numbers at the end go to stderr.
- The numbers above are the sender's alone. At 1280x720 a frame is 3.7 MB of escape codes with `direct`, 92 MB a second at 25 frames a second, which the terminal has to parse.
- `--scale` places the image over cells of 8x16 pixels, which is a guess at the font; `--cell` gives another.
- The `openh264` crate was given whole files, not a stream as it comes.
- No audio.

## Decision or next action

The decoder stays undecided. The choices and what speaks for each are with the project's owner.
