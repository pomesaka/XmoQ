# xmoq-watch

A viewer for the terminal: it subscribes to the video and the audio track of a broadcast on one MoQ Transport session, and plays both by the Timestamps of LOC: the picture with the Kitty graphics protocol (Ghostty, kitty), the sound on the machine's output. With the room of a broadcast it asks the room over its HTTP API (`fixtures/room-api/README.md`) where the relay and the broadcast are, gets a token for the relay there, and follows a broadcast that has not begun yet, or ends and begins again. A room of a meeting is not taken: a meeting has a broadcast for each participant. The chat of the room is on MoQ tracks, on the broadcast's session (below), and shown under the picture.

**Provisional throughout.** The video decoder is decided (`openh264`, 2026-10-13) and the sound output is not ([open questions](../../docs/open-questions.md)); the crate is built so that each can be changed in one file. **It is for Unix alone**: it sets the terminal's line mode with `termios`, and does not build for Windows. On macOS it was run; **on Linux it was run with no sound output** (by `mise run interop` in CI, and to a pseudo-terminal in a container, [record](../../docs/experiments/2026-10-24-one-video-track-to-many-viewers-on-linux.md#xmoq-watch-on-linux)): its sound output has not opened on Linux, and nobody saw its picture there.

```sh
mise run watch -- --room http://127.0.0.1:8787/broadcasts/demo # next to `mise run demo`, with its rooms
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
| `fit` | The cells of a terminal a picture takes. The decoder, which frames it is given, and by how much a picture is made smaller are [`xmoq-picture`](../picture/README.md)'s |
| `kitty` | The bytes written to the terminal |
| `tracks` | Which tracks of a catalog are played |
| `keys` | What a key asks for |
| `marks` | For measuring with the demo's publisher (`--measure-demo-marks`). `--clock-skew-ppm <n>`, also for measuring, has the viewer's clock run n parts in a million faster (`sound.rs`, `micros_since`) |

What the viewer shares with `xmoq-agent` is in two crates of its own. `xmoq-viewer` (`crates/viewer`) decides, without input or output: `room` (the requests and answers of the room's HTTP API, by the rows of `fixtures/room-api/`, and a line made safe for a terminal) and `chat_track` (the chat on MoQ tracks: the Groups of `said` and the lines that wait for the gatherer, which lines of `lines` are shown and in what order, and how a line is shown; the names and the JSON are `xmoq-chat-tracks`'s). `xmoq-viewer-io` (`crates/viewer-io`) does their input and output: `session.rs` (the requests on the MoQ Transport session, and the catalog), `room_client.rs` (the room's HTTP API, with a seat of a meeting renewed on a task of its own, which `xmoq-agent` takes), `chat_tracks.rs` (the chat on MoQ tracks).

The binary (`src/bin/xmoq-watch/`) does the rest of the input and output, one kind a file: `terminal.rs` (line mode, keys, the window's size), `picture.rs` (the thread that decodes and writes frames), `sound.rs` (the Opus decoder and the queue of the output), and `audio_output.rs` below.

## Dependencies that are not decided, and what they need

Read in the crates' manifests and build scripts at the versions of `Cargo.lock`, on 2026-10-12.

| Crate | Used for | In | License | Needs to build |
|---|---|---|---|---|
| `openh264` 0.9.8 (`openh264-sys2`), through `xmoq-picture` | Decoding H.264 | `crates/picture/src/video_decoder.rs` alone | BSD-2-Clause; it builds Cisco's OpenH264 from the source it carries (BSD-2-Clause) | A C++ compiler (`cc`). `nasm` is used for the assembly where it is found, and left out where it is not |
| `cpal` 0.18.2 | The sound output | `audio_output.rs` alone | Apache-2.0 | macOS: nothing more (CoreAudio through `coreaudio-rs` and the `objc2` crates). Linux: the headers of ALSA and `pkg-config` (`alsa-sys` 0.4.0 probes `alsa`; Debian and Ubuntu: `libasound2-dev`), which `scripts/ci/check-rust.sh` installs in CI and asks for elsewhere |
| `opus-rs` 0.1.39 (decided on 2026-10-24; why, in `sound.rs`) | Decoding Opus | `sound.rs` | BSD-3-Clause; a port of libopus 1.6 | Nothing but Rust. **Much `unsafe` of its own** (181 blocks and 71 functions in its `src/`, counted on 2026-10-21, not read), and a 0.x crate with 37 releases in eight months, whose API can change from one to the next: the version is pinned (`=0.1.39`) |
| `reqwest` 0.13.5 | The room's HTTP API | `room_client.rs` of `xmoq-viewer-io` | MIT OR Apache-2.0 | Nothing more than quinn needs (read on 2026-10-26): without its default features, so without aws-lc-rs; TLS is rustls with ring, which the MoQ connection already builds with a C compiler |
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
| The sound's least buffer | 30 ms | 30 ms, where `--buffer-ms` is less | The same rule (`AUDIO_MIN_US`) |

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
| The pixels of a decoded picture | 3840x2160; a larger one is not drawn (`pictures_too_large` in `AV_PLAYED`) | Four times 1080p. What the decoder holds itself for such a picture is not bounded here | `xmoq-picture` |
| Packets of sound that wait | 100 (2 s); the oldest goes | | `playout` |
| The relay's SETUP | 10 s (`--setup-wait-ms`); without it the session is closed with `CONTROL_MESSAGE_TIMEOUT` (`0x11`) | As `@xmoq/moqt` | `main.rs` |
| Connecting to the relay | 10 s (`--connect-wait-ms`, `CONNECT_WAIT_MS` of `xmoq_viewer::reconnect`, as `xmoq-agent` has it) | **Provisional** | `main.rs` |
| Each answer of a room (the room, a token), its body read | 5 s (`--room-wait-ms`, `ROOM_WAIT_MS` of `xmoq_viewer::reconnect`, as `xmoq-agent` has it), after the 30 s a room that refuses the connection is asked again for. The room is asked before each session; one that fails then ends the viewer | **Provisional**; a room can stand far from the viewer | `main.rs` |
| The next session | By `NextSession` of `xmoq-again`, whose rows are `fixtures/ask-again/next-session.tsv`. A first try that fails before anything played ends the viewer, with a room as well (`xmoq-agent` goes on). `createNextSession` of `@xmoq/moqt` reads the same rows, and `xmoq-agent`, the gatherer and the transcriber go by the same rule | | `main.rs` |
| The relay's answer to a SUBSCRIBE | 10 s from when it was written (`--answer-wait-ms`); the SUBSCRIBE alone is given up, reset and stopped with `CANCELLED`, and the viewer leaves or tries again | The row "A request the peer does not answer" of [the implementation notes](../../docs/moqt-implementation-notes.md), which the page and the Rust client go by | `session.rs` |
| A stream for a request, which the relay has to allow | 10 s (`--open-wait-ms`); the request is not sent, and the viewer leaves or tries again | The row "A request that waits for a stream" of the implementation notes | `session.rs` |
| The sound's subscription that ends while the video goes on | Asked for again on the same session after 1 s, or, after a refusal, after its Retry Interval (no less than 100 ms, doubling up to 5 s while refusals follow each other; no more than 30 s); the picture goes on. A GOAWAY on the subscription's stream (§10.4) ends it and has it asked for again on the session; a message on it that breaks the protocol closes the session. A SUBSCRIBE that could not be sent or got no answer is not tried again on that session (`AUDIO_ENDED`, `AUDIO_AGAIN`, `AUDIO_REFUSED`, `AUDIO_GIVEN_UP`, `AUDIO_SAID` in `--log`). `--cut-sound-s N`, for measuring, cancels the sound's subscription once N s after it was made | The rule and the values of the watching page (`keepTrackOnSession` of `@xmoq/media`); a relay ends the subscription of the one track that falls behind | `xmoq-again`, `xmoq_viewer_io::kept` |
| A broadcast that is not there yet | With a room, the catalog's SUBSCRIBE asks the relay to hold it for 30 s (`RENDEZVOUS_MS` of `xmoq-viewer-io`, the value for a followed track decided on 2026-10-26): the room names its broadcast whether it has begun or not, and says nothing when it begins. One refused at the end of that (`TIMEOUT`), or at once (`DOES_NOT_EXIST`), is asked again on the same session after the pause of `xmoq-again` (`ROOM_BROADCAST none: waiting, asking again in <ms> ms` in `--log`; `FollowedCatalog` of `xmoq-viewer-io`, as `xmoq-agent` has it), for as long as the viewer is not told to end, and is no failure of the session (`NextSession`). With `--video` and `--audio` the catalog is not read, and nothing is waited for. Without a room, refused at once and the end of the viewer; with `--rendezvous-timeout-ms N` the relay is asked to hold the viewer's SUBSCRIBEs for N ms (the own relay holds 30 s at most), and the viewer waits that much longer for each answer | **Provisional**. The row "A SUBSCRIBE for a track nobody publishes" of the implementation notes | `catalog.rs` of `xmoq-viewer-io`, `main.rs` |
| Requests the relay opens towards it that wait for their refusal | 16 in the queue, while the broadcast does not play yet; then the readers stop reading, and flow control holds the relay | Nothing is played before the SUBSCRIBEs are answered, and a relay here opens no request towards a viewer | `main.rs` |
| The catalog | 5 s as a whole, as the page does | Over before the FETCH for it could be given up | `session.rs` |
| What the session's unidirectional streams bring while an answer to a request is waited for | Read all the same: one that fails the session (a datagram of an unknown type, a stream that breaks the protocol) ends the wait and closes the session with its code; 256 others are kept for after (**provisional**), and beyond them reading pauses | An Object of a track can be read before the answer that subscribes to it | `session.rs` (`Events`) |
| An Object of the catalog's subscription, and of the chat's `lines` | 1 MiB; 64 KiB (**provisional**). Over it, that stream alone is stopped with `INTERNAL_ERROR`, decided at its Object Payload Length; the subscription and the session go on. An Object whose length was read before the SUBSCRIBE_OK is held to the 1 MiB undecoded of every subgroup stream | As the page (`@xmoq/moqt`'s `maxObjectPayloadLength`): what the viewer knows to be small is not collected up to the bound of every stream | `session.rs`, `chat_tracks.rs` |
| A room's token | 5 s | | `main.rs` |
| The chat on MoQ tracks (decided on 2026-10-24): on the broadcast's session, with `--room` the room of its address (`broadcast/<name>`) and with `--url` only with `--chat-room broadcast/<name>` or `meeting/<name>`, `--name` as the name. The viewer announces `xmoq/chat/<kind>/<room>/<name>`, writes each line it says (a key, or `--say`) as a Group of `said` for the gatherer, subscribes to `lines` of `xmoq/chat/<kind>/<room>` and takes the latest 16 lines by a Joining FETCH. The SUBSCRIBE carries a RENDEZVOUS_TIMEOUT of 30 s: the gatherer publishes `lines` of a room once a participant has come, so the first viewer of a room is held for it. A subscription that is refused or ends (the gatherer ends `lines` of a room nobody is in with `TRACK_ENDED`) is kept as the sound is (`xmoq_viewer_io::kept`), but for a SUBSCRIBE that could not be sent or got no answer, which is asked again as a refusal that names no time (`CHAT_TRACKS_LINES_FAILED`), as `@xmoq/chat` does; and its FETCH asks for what was missed: from the line after the newest shown, also on a session after another, to the new LARGEST_OBJECT; where that is more than `--chat-catch-up-lines` (16, as many as are kept; 1 to 1,000), the newest of them and a line that says how many were not fetched (`CHAT_GAP` in `--log`); and the newest 16 where the room was numbered anew (a LARGEST_OBJECT below the newest shown). The rule of `@xmoq/chat`'s `historyToFetch`, by the rows of `fixtures/chat-history/`. With a room that gives tokens, the viewer asks it for two more: `pub` in `xmoq/chat/<kind>/<room>/<name>`, which its PUBLISH_NAMESPACE carries, and `sub` in `xmoq/chat/<kind>/<room>/*`, which its SUBSCRIBE and FETCH of `lines` carry; the session's token, `sub` in the broadcast's namespace, is on its SETUP and its broadcast's requests alone. Each line is shown as its number, who said it and what (`CHAT ...` in `--log`); what fails of it is said (`CHAT_TRACKS_FAILED`) and the broadcast plays on | Lines said before the gatherer subscribes: 16 wait, a later one is not sent. A line over 512 bytes is not sent (the gatherer's `--text-bytes`). The FETCH's stream: 64 KiB. A stream for a line of `said`: 2 s to be opened, or it is not sent. **All provisional** | The gatherer subscribes when it is told of the namespace, after the viewer's first lines can be typed | `chat_track`, `chat_tracks.rs` |
| A room that refuses the connection | Asked again every 1 s (`xmoq-again`, a refusal that names no time) for 30 s from the first try (`ROOM_COMES_WITHIN`); another failure ends it at once | Rooms often start after their viewers (a demo, a Worker restarted) | `room_client.rs` |
| One answer of a room | 64 KiB | A seat with two tokens, the largest, is under 1 KiB | `room_client.rs` |
| Lines of the chat that are kept | 16 | 3 are shown | `main.rs` |
| A line of the chat, and any line the viewer writes | 120 and 2,000 characters, without control characters and without the marks that change the direction of text | A line of a terminal; `AV_PLAYED`, the longest line, is about 1,100 | `room::clean`, `main.rs` |
| From one try at a broadcast to the next | 1 s at least | A relay that fails at once | `main.rs` |
| Files of pixels with `--medium file` | 30 at once, each made anew for this user alone under a name that cannot be guessed; all removed at the end | A second of frames, for a terminal that removes them late or not at all | `picture.rs` |
| Values of each series that `AV_PLAYED` gives the spread of | The last 4,096 | About two minutes of frames | `marks::Recent` |

Whatever a peer is waited for, the viewer ends when it is told to: `q`, Ctrl-C, a signal and `--seconds` are looked at during every such wait (`Ui::unless_ended`). The exception is a terminal that does not take what is written: the thread of the picture is then given 2 s, and the viewer ends without it.

**What it cannot check**: a room reached over `http://` is not authenticated, so whoever is on the path can name another relay as the broadcast, and the viewer would send its token there. A room's address is accepted as it is given; a broadcast it names has to be `https://`.

## Checks

`mise run interop` runs the viewer (`interop/watch-rust.ts`; the runs are in the table of [`interop/README.md`](../../interop/README.md)), which CI runs on Linux: it finds its tracks in a catalog and plays them for 8 seconds with no terminal and no sound output (`--out /dev/null`, `--sound off`), of a page through the own relay and of moqlivemock's publisher among them. They hold what `AV_PLAYED` says against what is expected: the frames drawn, the packets of sound, the underruns, and with the page how far apart the marks of picture and sound leave the viewer.

What needs a sound output, the demo with its rooms, or a terminal is under `scripts/`, and is run by hand.

## Checks that need a person's machine

Under `scripts/`, from the repository root; each starts a demo of its own on other ports with its publishing page in headless Chromium (`mise run interop` once, for the browser). The picture goes to `/dev/null` or a pseudo-terminal and the sound is at volume 0: nobody sees or hears these runs.

| Script | What it shows |
|---|---|
| `try.sh [seconds]` | Picture and sound of the demo, found in its catalog: frames, underruns, how far apart the demo's marks leave the viewer, CPU and memory. `SIZE=1080` and `--fit 960x540` for a larger picture |
| `try-room.sh` | A room says where the broadcast is and gives the token, and the relay's refusal without a token. The viewer's `--say` goes to the chat's tracks, which show nothing where no gatherer runs, and the script starts none |
| `try-keys.sh` | The keys on a pseudo-terminal (`keys-on-a-pty.py`), a broadcast that ends and begins again, and the terminal's line mode after `q`, Ctrl-C and SIGTERM |
| `try-narrow-link.sh [seconds] [flags]` | The viewer behind a link of 2 Mbit/s with a picture of 2.5 Mbit/s (`spikes/lossy-link/proxy.mjs`): with `--video-timeout-ms 100` the relay gives up Groups of the video that are late, and the picture stays a quarter of a second behind ([record](../../docs/experiments/2026-10-12-the-terminal-viewer-asks-for-a-delivery-timeout.md)) With `PUBLISH_QUERY='bitrate=2500000&picture=busy&ladder=1'` the publisher has a second, small version, which `--video-version name:video-low` plays |
| `try-silent-peers.sh` | A room and a relay that never answer: the viewer ends with `--seconds`, with SIGTERM, and of itself after `--room-wait-ms` (5 s) for the room and 10 s for the relay. Needs no demo and no browser |

**After a panic** the terminal is put back by a hook; nothing checks it.

## What it does not do

- Decide anything about the video decoder and the output.
- Read a catalog that changes while it watches; choose between several video or audio tracks by anything but the first; follow a track into another namespace than the catalog's.
- Conceal a packet of sound that did not come; resample (an output that does not take 48 kHz ends it); play more than one channel (two are mixed into one by the decoder).
- Serve a request the relay opens towards it: each is refused with `NOT_SUPPORTED`, once the broadcast plays (until then up to 16 wait), and its Request ID is checked (§10.1), as the Rust client does in its modes that serve none.
- Run on Windows.
