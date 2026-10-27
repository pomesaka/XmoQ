# xmoq-watch

A viewer for the terminal: it subscribes to the video and the audio track of a broadcast on one MoQ Transport session, and plays both by the Timestamps of LOC: the picture with the Kitty graphics protocol (Ghostty, kitty), the sound on the machine's output. With the room of a broadcast it asks the room over its HTTP API (`fixtures/room-api/README.md`) where the relay and the broadcast are, gets a token for the relay there, and follows a broadcast that has not begun yet, or ends and begins again. A room of a meeting is not taken: a meeting has a broadcast for each participant.

The chat of the room is on MoQ tracks, on the broadcast's session, and shown under the picture: with `--room`, or with `--url` and `--chat-room`, the viewer takes part in it as `--name`. The tracks are [`xmoq-chat-tracks`](../chat-tracks/README.md)'s, and which of the past lines it fetches is the rule of [`fixtures/chat-history/`](../../fixtures/chat-history/README.md), as the pages' `followPlan` of `@xmoq/tracks`.

**Provisional throughout.** The sound output is not decided ([open questions](../../docs/open-questions.md)); the crate is built so that it can be changed in one file. **It is for Unix alone**: it sets the terminal's line mode with `termios`, and does not build for Windows. On macOS it was run; **on Linux it was run with no sound output** (by `mise run interop` in CI, and to a pseudo-terminal in a container, [record](../../docs/experiments/2026-10-24-one-video-track-to-many-viewers-on-linux.md#xmoq-watch-on-linux)): its sound output has not opened on Linux, and nobody saw its picture there.

```sh
mise run watch -- --room http://127.0.0.1:8787/broadcasts/demo # next to `mise run demo`, with its rooms
demo/broadcast/scripts/watch-demo.sh                            # the same demo without a room
mise run watch -- --url https://host:port/ --cert-sha256 <64 hex digits> --namespace a/b
```

`xmoq-watch` with a flag it does not know prints the flags and the keys (`USAGE` in `src/args.rs`). What runs it against a demo with a sound output, a room or a terminal is in `demo/broadcast/scripts/`, and is run by hand ([the terminal viewer beside a demo](../../demo/README.md#the-terminal-viewer-beside-a-demo)).

## What it needs to build

| Crate | Used for | In | License | Needs to build |
|---|---|---|---|---|
| `openh264`, through [`xmoq-picture`](../picture/README.md) | Decoding H.264 | `crates/picture/src/video_decoder.rs` alone | See `xmoq-picture` | A C++ compiler |
| `cpal` 0.18.2 | The sound output | `audio_output.rs` alone | Apache-2.0 | macOS: nothing more (CoreAudio through `coreaudio-rs` and the `objc2` crates). Linux: the headers of ALSA and `pkg-config` (`alsa-sys` 0.4.0 probes `alsa`; Debian and Ubuntu: `libasound2-dev`), which `scripts/ci/check-rust.sh` installs in CI and asks for elsewhere |
| `opus-rs` 0.1.39 (why, and what its `unsafe` is, in `sound.rs`) | Decoding Opus | `sound.rs` | BSD-3-Clause; a port of libopus 1.6 | Nothing but Rust |
| `reqwest` 0.13.5 | The room's HTTP API | `room_client.rs` of `xmoq-viewer-io` | MIT OR Apache-2.0 | Nothing more than quinn needs: without its default features, so without aws-lc-rs; TLS is rustls with ring, which the MoQ connection already builds with a C compiler |
| `rustix` 1.1.5 (why, in `terminal.rs`) | The terminal's line mode and the size of its window | `terminal.rs` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Nothing but Rust |

## Where it plays unlike the watching page

The buffer (`--buffer-ms`, and `--buffer-ms auto`) goes by the rule and the values of the watching page, and both players read the rows of [`fixtures/playout/`](../../fixtures/playout/README.md); the viewer adds what the sound output is ahead by (why, in `main.rs`). Where the two are not alike, and why:

| | The page | The viewer | Why |
|---|---|---|---|
| The unit | milliseconds, with fractions | whole microseconds | The page's clock counts milliseconds; LOC's Timestamps are microseconds. The shared rows are whole milliseconds, where both give the same |
| Which Object the rule of `auto-buffer.tsv` takes | counted in floating point, rounded down | counted in whole numbers | The same for every count the tests have; a floating product just under a whole number would be one Object apart |
| The lead of the sound output | none: the page's sound is timed by the `AudioContext` | added to the buffer, for picture and sound alike | The output here asks for samples ahead of time. `AV_PLAYED` has the buffer without it (`buffer_ms`, `buffer_ms_least/most`) |
| When a frame's time is fixed | when it is drawn: a buffer that moves moves the frames that wait | when it comes | The thread that draws has the frame's moment and not the clock; a step of the buffer (`AUTO_STEP_US` of `src/clock.rs`) is a third of a frame |
| A packet whose beginning is over at the output | not sounded | played from the sample that is due (`audio_cut_late`) | The output asks for samples ahead of time, so a packet that was placed in time can still be late there; cut, the sound does not stay behind |
| Slower or faster | the browser's `playbackRate` | samples on the lines between the packet's samples | No resampler is among the crates; how much slower or faster is the rule of `seam.tsv` |
| Packets out of order | held until the ones before them came (`holdForOrderMs`) | played as they come | Not built. A stream keeps the order within a Group; between Groups nothing does |

The viewer does not change the version of the video by itself: `--video-version` chooses one where the catalog has several ([record](../../docs/experiments/2026-10-12-the-terminal-viewer-chooses-a-version-of-the-video.md)). Behind a link with less room than the video, `--video-timeout-ms` keeps the picture from falling behind ([record](../../docs/experiments/2026-10-12-the-terminal-viewer-asks-for-a-delivery-timeout.md)).

Every limit on what the relay, the publisher behind it and the room can have it hold or wait for is a constant, with its value and its reason, in `src/`, `src/bin/xmoq-watch/`, `xmoq-viewer`, `xmoq-viewer-io` and `xmoq-subscriber`; those the pages share are the rows of the implementation notes and of `fixtures/ask-again/`, `fixtures/kept-track/` and `fixtures/followed-track/`.

**What it cannot check**: a room reached over `http://` is not authenticated, so whoever is on the path can name another relay as the broadcast, and the viewer would send its token there. A room's address is accepted as it is given; a broadcast it names has to be `https://`.

**After a panic** the terminal is put back by a hook; nothing checks it.

## What it does not do

- Decide anything about the sound output.
- Read a catalog that changes while it watches; choose between several video or audio tracks by anything but the first; follow a track into another namespace than the catalog's.
- Conceal a packet of sound that did not come; resample (an output that does not take 48 kHz ends it); play more than one channel (two are mixed into one by the decoder).
- Join the video before its next key frame: it sends neither a Joining FETCH nor a NEW_GROUP_REQUEST for it.
- Serve a request the relay opens towards it: each is refused with `NOT_SUPPORTED`, once the broadcast plays (until then they wait, as many as `read_peer_streams` of `xmoq-subscriber` holds), and its Request ID is checked (§10.1), as the Rust client does in its modes that serve none.
- Run on Windows.
