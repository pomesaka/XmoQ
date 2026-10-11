# A broadcast in a terminal, picture and sound, with one command

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal demo

## Objective

One command that plays the broadcast of `mise run demo` in a terminal: the picture with the Kitty graphics protocol, the sound through the machine's output. **An experiment, and no choice of a decoder or of an audio backend**, which are undecided ([open questions](../open-questions.md)): each part is the one with the fewest steps to try.

## Environment

macOS, loopback. The own relay and the Rust client of the working copy this record is part of; Chromium 156, headless, for the pages that publish; ffmpeg 8.1. **No terminal and no loudspeaker**: the picture went to a file, and the sound to a file or out at volume 0.

## Commands

```sh
spikes/terminal-player/play.sh          # in Ghostty, with `mise run demo` up and /publish open
```

- Picture: the Rust client subscribes to the track `video` and writes Annex B into a FIFO; `kitty-frames` (`spikes/kitty-graphics`) decodes it with the `openh264` crate and draws it.
- Sound: a second session subscribes to the track `audio`; `opus-ogg` (`spikes/opus-ogg`) wraps the Opus packets as Ogg as they come, and `ffmpeg -f ogg -i pipe:0 -f audiotoolbox -` decodes and plays them. `audiotoolbox` is an output of ffmpeg on macOS.
- It takes the relay of the demo (`https://127.0.0.1:4443/`, the namespace `xmoq/demo`, the certificate of `demo/.cert`), or `URL`, `CERT_SHA256` and `NAMESPACE` from the environment. No catalog.

What was run here, in place of the demo's pages: the relay and two headless pages of `interop/` in the namespace `xmoq/video`, one publishing video (bars, 320x180) and one a tone as Opus (48,000 Hz, one channel), and `play.sh` with `FRAMES=100 SOUND=<file>`.

## Observed results

One run.

- **The page's audio reached the Rust client through the own relay**: 200 Objects, written as Ogg Opus; `ffprobe` reads `opus`, 48,000 Hz, 1 channel, 4.000000 seconds.
- The picture at the same time, from the other page through the same relay: 100 of 100 frames decoded and handed to be drawn, in 3.7 seconds.
- `ffmpeg` took the Ogg file from a pipe and played it through `audiotoolbox` at volume 0 for half a second, with the exit code 0.

## Limitations and blockers

- **Nobody saw the picture or heard the sound**, and the command was not run against the pages of `demo/`, which need a person to open them in Chromium. Their track names, namespace and certificate were read in `demo/shared.ts` and `demo/serve.ts`.
- **Picture and sound are not held to each other's times.** Each is played as it comes: the sound through ffmpeg, which buffers before it plays, so it is later than the picture by some part of a second that was not measured.
- The sound works on macOS alone.
- The first attempt hung: with one page publishing both tracks, the page of `interop/` answered for the audio alone and the relay turned the SUBSCRIBE for the video down, and the program that draws waited for ever to open the FIFO. The script now stops it when the client was turned down, and says so.
- The pre-skip of the Ogg stream is a guess (312 samples), as in [the record of the Opus file](2026-10-11-rust-subscriber-writes-opus-as-ogg.md).

## Decision or next action

None. The decoders stay undecided.
