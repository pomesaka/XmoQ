# xmoq-watch

A viewer for the terminal: it subscribes to the video and the audio track of a broadcast on one MoQ Transport session, and plays both by the Timestamps of LOC: the picture with the Kitty graphics protocol (Ghostty, kitty), the sound on the machine's output. With a room it learns from the room where the broadcast is, gets a token for the relay there, shows the room's chat under the picture, and follows a broadcast that ends and begins again.

**Provisional throughout.** The decoders and the sound output are not decided ([open questions](../../docs/open-questions.md)); the crate is built so that each can be changed in one file. It runs on Unix alone: it sets the terminal's line mode with `termios`. On macOS it was run; **on Linux it has only been built by CI, and nothing of it was run there.**

```sh
cargo run --release -p xmoq-watch -- --room http://127.0.0.1:8787/rooms/demo
cargo run --release -p xmoq-watch -- --url https://127.0.0.1:4443/ --cert-sha256 <64 hex digits>
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

## What it does not do

- Decide anything about the decoders and the output, or about the buffer: `--buffer-ms` is 50, the watching page's default, and does not follow how late Objects come.
- Read a catalog that changes while it watches; choose between several video or audio tracks by anything but the first; follow a track into another namespace than the catalog's.
- Conceal a packet of sound that did not come; resample (an output that does not take 48 kHz ends it); play more than one channel (two are mixed into one by the decoder).
- Reach a room over TLS (`wss://`).
- Answer a request the relay opens towards it.
- Run on Windows.
