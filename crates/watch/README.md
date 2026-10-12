# xmoq-watch

A viewer for the terminal: it subscribes to the video and the audio track of a broadcast on one MoQ Transport session, and plays both by the Timestamps of LOC: the picture with the Kitty graphics protocol (Ghostty, kitty), the sound on the machine's output. With a room it learns from the room where the broadcast is, gets a token for the relay there, shows the room's chat under the picture, and follows a broadcast that ends and begins again.

**Provisional throughout.** The video decoder and the sound output are not decided ([open questions](../../docs/open-questions.md)); the crate is built so that each can be changed in one file. **It is for Unix alone**: it sets the terminal's line mode with `termios`, and does not build for Windows. On macOS it was run; **on Linux it has only been built by CI, and nothing of it was run there.**

```sh
mise run watch -- --room http://127.0.0.1:8787/rooms/demo      # next to `mise run demo`, with its rooms
crates/watch/scripts/watch-demo.sh                              # the same demo without a room
mise run watch -- --url https://host:port/ --cert-sha256 <64 hex digits> --namespace a/b
```

`xmoq-watch` with a flag it does not know prints the flags and the keys (`USAGE` in `src/args.rs`).

## What is where

The library (`src/*.rs`) decides, and does no input or output: it is given its times and its bytes, and has the tests.

| Module | What it decides |
|---|---|
| `args` | What the flags mean |
| `clock` | When an Object is due, from its Timestamp of LOC |
| `playout` | Which sample the sound output gets next |
| `fit` | By how much a picture is made smaller for a terminal, and the picture that is |
| `kitty` | The bytes written to the terminal |
| `tracks` | Which tracks of a catalog are played |
| `keys` | What a key asks for |
| `room` | What a room says and is told; a line of the chat made safe for a terminal |
| `marks` | For measuring with the demo's publisher (`--measure-demo-marks`) |

The binary (`src/bin/xmoq-watch/`) does the input and output, one kind a file: `session.rs` (the requests on the MoQ Transport session), `room_socket.rs` (the room's WebSocket), `terminal.rs` (line mode, keys, the window's size), `picture.rs` (the thread that decodes and writes frames), `sound.rs` (the Opus decoder and the queue of the output), and the two files below.

## Dependencies that are not decided, and what they need

Read in the crates' manifests and build scripts at the versions of `Cargo.lock`, on 2026-10-12.

| Crate | Used for | In | License | Needs to build |
|---|---|---|---|---|
| `openh264` 0.9.8 (`openh264-sys2`) | Decoding H.264 | `video_decoder.rs` alone | BSD-2-Clause; it builds Cisco's OpenH264 from the source it carries (BSD-2-Clause) | A C++ compiler (`cc`). `nasm` is used for the assembly where it is found, and left out where it is not |
| `cpal` 0.18.2 | The sound output | `audio_output.rs` alone | Apache-2.0 | macOS: nothing more (CoreAudio through `coreaudio-rs` and the `objc2` crates). Linux: the headers of ALSA and `pkg-config` (`alsa-sys` 0.4.0 probes `alsa`; Debian and Ubuntu: `libasound2-dev`), which `scripts/ci/check-rust.sh` installs in CI and asks for elsewhere |
| `opus-decoder` 0.1.1 | Decoding Opus | `sound.rs` | MIT OR Apache-2.0 | Nothing but Rust |
| `tokio-tungstenite` 0.30.0 | The room's WebSocket | `room_socket.rs` | MIT | Nothing but Rust. Without TLS: a room is reached over `ws://` alone |
| `rustix` 1.1.5 | The terminal's line mode and the size of its window | `terminal.rs` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Nothing but Rust |

WHY `rustix`: the workspace forbids `unsafe`, and `termios` and the window's size are system calls. `rustix` has them as safe functions. `crossterm` would do as well, and brings its own event loop and screen handling for the two calls that are needed; it was not tried.

## The buffer

Every Object is due `--buffer-ms` (50, the watching page's default; provisional) after the fastest way an Object was seen to take, **plus what the sound output is ahead by**: the output asks for a sample some time before it leaves the machine (how far ahead it asks, and the length of one call; 24.3 ms on the Mac this was written on), and that time is added to the picture's and the sound's due times alike. WHY: without it that much of the buffer is gone before a packet may be late at all, and 50 ms here would be less than 50 ms in the page. The lead is read once, when a broadcast begins to play; `AV_PLAYED` has it (`output_lead_ms`).

## Checks that need a person's machine

Under `scripts/`, from the repository root; each starts a demo of its own on other ports with its publishing page in headless Chromium (`mise run interop` once, for the browser). The picture goes to `/dev/null` or a pseudo-terminal and the sound is at volume 0: nobody sees or hears these runs.

| Script | What it shows |
|---|---|
| `try.sh [seconds]` | Picture and sound of the demo, found in its catalog: frames, underruns, how far apart the demo's marks leave the viewer, CPU and memory. `SIZE=1080` and `--fit 960x540` for a larger picture |
| `try-room.sh` | A room says where the broadcast is and gives the token, chat both ways, and the relay's refusal without a token |
| `try-keys.sh` | The keys on a pseudo-terminal (`keys-on-a-pty.py`), a broadcast that ends and begins again, and the terminal's line mode after `q`, Ctrl-C and SIGTERM |

WHY not in `interop/`: they need a sound output, which the machines of CI have none of, and the demo's pages, which the interop runs do not start. **After a panic** the terminal is put back by a hook; that was run by hand on 2026-10-12 through a switch that is no longer in the binary, and nothing checks it now.

## What it does not do

- Decide anything about the video decoder and the output.
- Follow how late Objects come with its buffer, as the watching page's playout does: `--buffer-ms` is what it is given. What the page looks at and how much it adds are provisional themselves, and the two are to be made alike before this is built.
- Read a catalog that changes while it watches; choose between several video or audio tracks by anything but the first; follow a track into another namespace than the catalog's.
- Conceal a packet of sound that did not come; resample (an output that does not take 48 kHz ends it); play more than one channel (two are mixed into one by the decoder).
- Reach a room over TLS (`wss://`).
- Answer a request the relay opens towards it.
- Run on Windows.
