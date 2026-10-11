# A Rust subscriber writes an Opus track of LOC as an Ogg Opus file

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, native audio output

## Objective

To see that the Objects of an audio track the Rust client takes are sound a decoder reads, with as little as that takes. The audio decoder of the CLI is undecided ([open questions](../open-questions.md)), and **nothing here decides it**: the spike decodes nothing, and what read its file was ffmpeg.

## Environment

macOS, loopback. The Rust client of the working copy this record is part of; `mlmpub` of moqlivemock v0.16.1, whose catalog has the track `audio_monotonic_128kbps_opus` (`opus`, 48,000 Hz, `channelConfig` "2"); `ffprobe` and `ffmpeg` 8.1. `spikes/opus-ogg`, a Rust program without dependencies.

## Commands

```sh
spikes/opus-ogg/from-mlmpub.sh audio.opus 250
ffprobe -v error -show_entries stream=codec_name,sample_rate,channels,duration -of csv=p=0 audio.opus
ffmpeg -i audio.opus audio.wav && ffmpeg -i audio.wav -af volumedetect -f null -
```

The script has the client subscribe to the track and print its Objects; `opus-ogg` reads the `OBJECT` lines and puts the payload of each, one Opus packet, into an Ogg page of its own (RFC 7845), with the number of samples it reads from the first byte of the packet (RFC 6716, §3.1).

## Observed results

One run.

- 250 Objects, 80,250 bytes of payload, each with a Timestamp of LOC 20,000 microseconds after the one before it (4,980,000 from the first to the last).
- `ffprobe`: `opus`, 48,000 Hz, 2 channels, 5.000000 seconds.
- `ffmpeg` decoded it to 4.9935 seconds of PCM without a warning; `volumedetect` has a mean of -30.3 dB and a highest of -22.7 dB, so it is not silence.

## Limitations and blockers

- **Nobody heard it.** `ffplay audio.opus` plays the file.
- **No relay and no page**: `mlmpub` is a server and connects to no relay, and the pages of `interop/` publish no audio in this working copy (no `publishAudio`). The path through the own relay was not run for audio.
- The pre-skip of the file is 312 samples, a guess: neither LOC nor the catalog says what the encoder used. The first milliseconds of the file may be cut wrongly.
- The track was named on the command line.
- Not decoded in Rust. The choices for that, none tried: the crate `opus` or `audiopus` (bindings to libopus, BSD, a C library to build or to find on the machine); `symphonia` (pure Rust, MPL-2.0; whether it decodes Opus was not checked); ffmpeg as a library or as a process, as for video.

## Decision or next action

None. The audio decoder stays undecided.
