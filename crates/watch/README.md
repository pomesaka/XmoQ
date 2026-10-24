# xmoq-watch

A viewer for the terminal: it subscribes to the video and the audio track of a broadcast on one MoQ Transport session, and plays both by the Timestamps of LOC: the picture with the Kitty graphics protocol (Ghostty, kitty), the sound on the machine's output. With a room it learns from the room where the broadcast is, gets a token for the relay there, shows the room's chat under the picture, and follows a broadcast that ends and begins again.

**Provisional throughout.** The video decoder is decided (`openh264`, 2026-10-13) and the sound output is not ([open questions](../../docs/open-questions.md)); the crate is built so that each can be changed in one file. **It is for Unix alone**: it sets the terminal's line mode with `termios`, and does not build for Windows. On macOS it was run; **on Linux it was run with no sound output** (by `mise run interop` in CI, and to a pseudo-terminal in a container, [record](../../docs/experiments/2026-10-24-one-video-track-to-many-viewers-on-linux.md#xmoq-watch-on-linux)): its sound output has not opened on Linux, and nobody saw its picture there.

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
| `marks` | For measuring with the demo's publisher (`--measure-demo-marks`). `--clock-skew-ppm <n>`, also for measuring, has the viewer's clock run n parts in a million faster (`sound.rs`, `micros_since`) |

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

Every Object is due `--buffer-ms` (150, the watching page's default, decided on 2026-10-21; the reasons are with `DEFAULT_PLAYOUT_BUFFER_MS` in `packages/media/src/core/playout-clock.ts`) after the fastest way an Object was seen to take, **plus what the sound output is ahead by**: the output asks for a sample some time before it leaves the machine (how far ahead it asks, and the length of one call; 24.3 ms on the Mac this was written on), and that time is added to the picture's and the sound's due times alike. WHY: without it that much of the buffer is gone before a packet may be late at all, and 150 ms here would be less than 150 ms in the page. The lead is read once, when a broadcast begins to play; `AV_PLAYED` has it (`output_lead_ms`).

**`--buffer-ms auto` (a trial, not the default)** has the buffer follow how late Objects come, by the rule of the watching page's `?buffer=auto` and with its values (`packages/media/src/core/playout-clock.ts`; all provisional): what 99 of 100 Objects of the last 10 seconds came within, after the fastest, and 10 ms more, in steps of 10 ms, from 150 up to 400 ms. It grows at once and shrinks 10 ms twice a second; fewer than 50 Objects move nothing. **A packet of sound follows the one before it to the sample**, and where the buffer has moved its time by more than 8 ms it is played up to a twentieth slower or faster until the sound is at its own time again (`playout::place`, the page's `placePiece`); this holds with a buffer that is set as well. Both players read the same rows of [`fixtures/playout/`](../../fixtures/playout/README.md) in their tests.

Behind a narrow link it kept the sound where 50 ms did not, and went to its limit of 400 ms, since the frames count as the packets do and came later ([record](../../docs/experiments/2026-10-12-the-terminal-viewer-has-the-pages-automatic-buffer.md)). Nobody has listened to it.

Where the two are not alike, and why:

| | The page | The viewer | Why |
|---|---|---|---|
| The unit | milliseconds, with fractions | whole microseconds | The page's clock counts milliseconds; LOC's Timestamps are microseconds. The shared rows are whole milliseconds, where both give the same |
| Which of the Objects "99 of 100" is | the one at `floor(count * 0.99)` in order, counted in floating point | the one at `count * 99 / 100`, in whole numbers | The same for every count the tests have; a floating product just under a whole number would be one Object apart |
| The lead of the sound output | none: the page's sound is timed by the `AudioContext` | added to the buffer, for picture and sound alike (above) | The output here asks for samples ahead of time. `AV_PLAYED` has the buffer without it (`buffer_ms`, `buffer_ms_least/most`) |
| When a frame's time is fixed | when it is drawn: a buffer that moves moves the frames that wait | when it comes | The thread that draws has the frame's moment and not the clock; a step is 10 ms, a third of a frame |
| A packet whose beginning is over at the output | not sounded | played from the sample that is due (`audio_cut_late`) | The output asks for samples ahead of time, so a packet that was placed in time can still be late there; cut, the sound does not stay behind |
| Slower or faster | the browser's `playbackRate` | samples on the lines between the packet's samples | No resampler is among the crates; a twentieth for one packet |
| Packets out of order | held until the ones before them came (`holdForOrderMs`) | played as they come | Not built. A stream keeps the order within a Group; between Groups nothing does |
| The sound's least buffer | 30 ms | 30 ms, where `--buffer-ms` is less | The same rule (`AUDIO_MIN_US`); before, the viewer had none |

**Which version of the video**: where the catalog has several H.264 tracks (the demo's `/publish?ladder=1`), `--video-version` says which is played: `first` (the default), `name:<track>`, or the largest within `height:<n>` or `bitrate:<n>`, and the smallest where none is within it. On a link of 2 Mbit/s the small version of the demo came whole and 10 ms after the fastest, where the large one lost a quarter of its frames and came 300 ms late ([record](../../docs/experiments/2026-10-12-the-terminal-viewer-chooses-a-version-of-the-video.md)). The viewer does not change the version by itself.

Behind a link with less room than the video, 50 ms is not enough for the sound: packets come 50 to 100 ms later than the fastest there, and `--buffer-ms 300` was what played all of it ([record](../../docs/experiments/2026-10-12-the-terminal-viewer-asks-for-a-delivery-timeout.md)). `--video-timeout-ms N` asks the relay to give up Groups of the video that are not delivered N ms after their last Object, which keeps the picture from falling behind there.

## What a peer cannot make it do without end

The relay, the publisher behind it and the room all say things the viewer acts on. Each has a limit; **every value is provisional**, chosen to be far from what the demo does and near enough to stop.

| What | Limit | Why that value | Where |
|---|---|---|---|
| A Timestamp far from the clock (the fastest way an Object took) | 10 s either way; the Object is left out and the clock stays. After 30 in a row the clock begins anew. The fastest way is that of the last 60 s (`LEAST_WINDOW_SECONDS`, the page's rule and value; `fixtures/playout/least.tsv`), so that a clock that runs faster than the publisher's does not lose its sound in time | Far beyond a buffer and beyond what one waits for a live picture; 30 is a second of video, for a publisher that began again | `clock` |
| `--buffer-ms` | 10,000; `auto` goes to 400 ms | The same 10 s; the page's limit | `args`, `clock` |
| Arrivals the automatic buffer goes by | 1,600, the newest | The page's: 10 seconds of 30 frames and 50 packets, twice over | `clock` |
| Frames that wait to be decoded and drawn | 30; one more is left out, and the picture goes on at the next Group | What the watching page holds | `picture.rs` |
| The pixels of a decoded picture | 3840x2160; a larger one is not drawn (`pictures_too_large` in `AV_PLAYED`) | Four times the largest the demo sends. What the decoder holds itself for such a picture is not bounded here | `fit`, `video_decoder.rs` |
| Packets of sound that wait | 100 (2 s); the oldest goes | | `playout` |
| The relay's SETUP; connecting to it; a room's hello | 10 s each; without a SETUP the session is closed with `CONTROL_MESSAGE_TIMEOUT` (`0x11`) | As `@xmoq/moqt` | `main.rs` |
| The relay's answer to a SUBSCRIBE | 10 s from when it was written (`--answer-wait-ms`); the SUBSCRIBE alone is given up, reset and stopped with `CANCELLED`, and the viewer leaves or tries again | The row "A request the peer does not answer" of [the implementation notes](../../docs/moqt-implementation-notes.md), which the page and the Rust client go by | `session.rs` |
| A stream for a request, which the relay has to allow | 10 s (`--open-wait-ms`); the request is not sent, and the viewer leaves or tries again | The row "A request that waits for a stream" of the implementation notes | `session.rs` |
| The sound's subscription that ends while the video goes on | Asked for again on the same session after 1 s, or, after a refusal, after its Retry Interval (no less than 100 ms, doubling up to 5 s while refusals follow each other; no more than 30 s); the picture goes on. A SUBSCRIBE that could not be sent or got no answer is not tried again on that session (`AUDIO_ENDED`, `AUDIO_AGAIN`, `AUDIO_REFUSED`, `AUDIO_GIVEN_UP` in `--log`). `--cut-sound-s N`, for measuring, cancels the sound's subscription once N s after it was made | The rule and the values of the watching page (`keepTrackOnSession` of `@xmoq/media`); a relay ends the subscription of the one track that falls behind | `again`, `main.rs` |
| A broadcast that is not there yet | Refused at once; with `--rendezvous-timeout-ms N` the relay is asked to hold the viewer's SUBSCRIBEs for N ms (the own relay holds 30 s at most), and the viewer waits that much longer for each answer | The row "A SUBSCRIBE for a track nobody publishes" of the implementation notes; without a room the viewer could not come first before | `session.rs` |
| The catalog | 5 s as a whole, as the page does | Over before the FETCH for it could be given up | `session.rs` |
| A room's token, and its taking a line of the chat | 5 s | | `main.rs` |
| One message of a room | 256 KiB | A hello with a full history is under 128 KiB | `room_socket.rs` |
| Lines of a room's history that are read; lines of the chat that are kept | 64; 16 | The room keeps 50; 3 are shown | `room`, `main.rs` |
| What a room says while a token is waited for | 256 messages; the oldest goes | | `room_socket.rs` |
| A line of the chat, and any line the viewer writes | 120 and 2,000 characters, without control characters and without the marks that change the direction of text | A line of a terminal; `AV_PLAYED`, the longest line, is about 1,100 | `room::clean`, `main.rs` |
| From one try at a broadcast to the next | 1 s at least | A room that names one broadcast after the other | `main.rs` |
| Files of pixels with `--medium file` | 30 at once, each made anew for this user alone under a name that cannot be guessed; all removed at the end | A second of frames, for a terminal that removes them late or not at all | `picture.rs` |
| Values of each series that `AV_PLAYED` gives the spread of | The last 4,096 | About two minutes of frames | `marks::Recent` |

Whatever a peer is waited for, the viewer ends when it is told to: `q`, Ctrl-C, a signal and `--seconds` are looked at during every such wait (`Ui::unless_ended`). The exception is a terminal that does not take what is written: the thread of the picture is then given 2 s, and the viewer ends without it.

**What it cannot check**: a room reached over `ws://` is not authenticated, so whoever is on the path can name another relay as the broadcast, and the viewer would send its token there. A room's address is accepted as it is given; a broadcast it names has to be `https://`.

## Checks

`mise run interop` has two runs with the viewer (`interop/watch-rust.ts`), which CI runs on Linux: it finds its tracks in a catalog and plays them for 8 seconds with no terminal and no sound output (`--out /dev/null`, `--sound off`), of a page through the own relay and of moqlivemock's publisher. They hold what `AV_PLAYED` says against what is expected: the frames drawn, the packets of sound, the underruns, and with the page how far apart the marks of picture and sound leave the viewer.

What needs a sound output, the demo with its rooms, or a terminal is under `scripts/`, and is run by hand.

## Checks that need a person's machine

Under `scripts/`, from the repository root; each starts a demo of its own on other ports with its publishing page in headless Chromium (`mise run interop` once, for the browser). The picture goes to `/dev/null` or a pseudo-terminal and the sound is at volume 0: nobody sees or hears these runs.

| Script | What it shows |
|---|---|
| `try.sh [seconds]` | Picture and sound of the demo, found in its catalog: frames, underruns, how far apart the demo's marks leave the viewer, CPU and memory. `SIZE=1080` and `--fit 960x540` for a larger picture |
| `try-room.sh` | A room says where the broadcast is and gives the token, chat both ways, and the relay's refusal without a token |
| `try-keys.sh` | The keys on a pseudo-terminal (`keys-on-a-pty.py`), a broadcast that ends and begins again, and the terminal's line mode after `q`, Ctrl-C and SIGTERM |
| `try-narrow-link.sh [seconds] [flags]` | The viewer behind a link of 2 Mbit/s with a picture of 2.5 Mbit/s (`spikes/lossy-link/proxy.mjs`): with `--video-timeout-ms 100` the relay gives up Groups of the video that are late, and the picture stays a quarter of a second behind ([record](../../docs/experiments/2026-10-12-the-terminal-viewer-asks-for-a-delivery-timeout.md)) With `PUBLISH_QUERY='bitrate=2500000&picture=busy&ladder=1'` the publisher has a second, small version, which `--video-version name:video-low` plays |
| `try-silent-peers.sh` | A room and a relay that never answer: the viewer ends with `--seconds`, with SIGTERM, and of itself after 10 s. Needs no demo and no browser |

**After a panic** the terminal is put back by a hook; that was run by hand on 2026-10-12 through a switch that is no longer in the binary, and nothing checks it now.

## What it does not do

- Decide anything about the video decoder and the output.
- Read a catalog that changes while it watches; choose between several video or audio tracks by anything but the first; follow a track into another namespace than the catalog's.
- Conceal a packet of sound that did not come; resample (an output that does not take 48 kHz ends it); play more than one channel (two are mixed into one by the decoder).
- Reach a room over TLS (`wss://`).
- Answer a request the relay opens towards it.
- Run on Windows.
