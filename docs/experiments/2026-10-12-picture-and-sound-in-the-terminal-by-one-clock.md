# Picture and sound in the terminal by one clock

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal

## Objective

The picture (`spikes/kitty-graphics`) and the sound (`spikes/rust-audio`) of the terminal were two processes, each playing what came as it came. Whether one Rust program can take both tracks on one session and put them out by the Timestamps of LOC, and how far apart picture and sound leave it.

## Environment

One machine, macOS on Apple silicon, loopback. The demo of this working copy on ports of its own (`demo/serve.ts`: `xmoq-relay` and the pages), its publishing page in headless Chromium: the clock's picture, 640x360, 30 frames a second, H.264, and the tone, Opus, one channel, packets of 20 ms. The viewer is `spikes/terminal-av` (`xmoq-watch`), `--release`, Rust 1.99.0.

## What was built

`xmoq-watch` uses `xmoq-moqt`, `xmoq-moqt-io` and `xmoq-loc` itself; nothing in `crates/` was changed. It opens one session, sends two SUBSCRIBEs, and goes on until Ctrl-C, `--seconds`, or the end of a track.

- **The clock**: the smallest (arrival - Timestamp) seen on either track is taken for the least an Object needs, and every Object is due `--buffer-ms` (50, the default of the watching page) after its Timestamp plus that.
- **Picture**: the crate `openh264` decodes a frame as it comes; a thread holds it until it is due and writes it with the Kitty graphics protocol (`--medium file`, the default, a temporary file the terminal reads; `direct`, the pixels in the escape codes).
- **Sound**: the crate `opus-decoder` decodes a packet as it comes; the callback of `cpal` plays each packet at the moment it is due, by the time `cpal` says the samples leave the machine. A packet due within 10 ms of where the output is, is played at once; one that is later begins at the sample that is due; silence where nothing is due yet.
- **The tracks are named by flags** (`--video video --audio audio`), **not found in a catalog**: the demo's pages publish no catalog ("What the two pages of the demo agree on without a catalog", `demo/shared.ts`).

## Commands

```sh
spikes/terminal-av/try.sh 20                  # the measurement: a demo of its own, headless
spikes/terminal-av/watch.sh                   # for a person: the broadcast of `mise run demo`, in Ghostty
```

`try.sh` writes the picture to `/dev/null` with `--medium direct` and plays the sound at volume 0. **Nobody saw the picture in a terminal and nobody listened.** What was written was looked at once in a file: `ESC _G a=T,f=24,s=640,v=360,...` for every frame, 0.92 MB a frame with `direct`; with `file`, 178 bytes a frame and one file of pixels a frame.

How far apart picture and sound are is measured at the output by the mark of the demo: for the first 200 ms of every 2 seconds the picture has a white square and the sound a loud beep. The viewer notes when the first frame with the square was written and when the first packet with the beep is played.

## Observed results

Three runs of 20 seconds, the buffer at 50 ms in the first two and 0 in the third.

| | 50 ms, run 1 | 50 ms, run 2 | 0 ms |
|---|---|---|---|
| Frames that came / drawn | 607 / 607 | 607 / 607 | 607 / 607 |
| Frames a second | 30.3 | 30.4 | 30.3 |
| A frame written after it was due, ms (median / max) | 7.2 / 14.4 | 7.2 / 11.4 | 15.7 / 38.9 |
| Packets of sound decoded | 1,002 | 1,001 | 1,001 |
| Sound played, seconds | 20.01 | 20.00 | **0.13** |
| Underruns | 0 | 0 | 1,874 |
| A packet played after it was due, ms (median) | 2.7 | -5.1 | |
| Marks compared | 10 | 11 | 10 |
| Picture after sound, ms (least / median / most) | -15.0 / -6.1 / 14.1 | -39.8 / -4.5 / 13.1 | |
| CPU (user + sys) of 20 s | 3.04 s | 3.25 s | 3.82 s |
| Maximum resident set | 28.5 MB | 28.3 MB | 28.3 MB |

- **With 50 ms picture and sound leave the program within about a frame of each other**: the median is 5 to 6 ms, the picture first; a mark can be told to a frame (33 ms) and a packet (20 ms) at best. One mark of 21 was 40 ms apart.
- All frames are drawn, at the rate they are made, and the sound has no underrun.
- **With 0 ms there is no sound**: the samples a callback fills leave the machine later than the call, so a packet has to be there before it is due by that much; 913 of 1,001 packets were over when their turn came. The picture does not mind.
- Decoding a frame takes 0.7 to 0.8 ms and writing it for the terminal about 0.9 ms (to `/dev/null`); 15% of one core for both tracks. What the terminal spends on drawing is not in it.
- Before a correction the sound was 46 ms behind the picture for a whole run: what comes first after SUBSCRIBE is the Group under way, up to 200 ms old, and it was played from its beginning. A late packet now begins at the sample that is due.

## Not done

- Looking at it in Ghostty, and listening.
- A catalog (see above), and tracks whose Timestamps have a Timescale.
- A link that delays or loses; a buffer that follows how late things come; concealing a packet that did not come.
- Requests the relay opens towards the viewer are not read.
- Linux and Windows.

## Next action

A person runs `spikes/terminal-av/watch.sh` in Ghostty next to `mise run demo`. The decoders and the output stay open questions.
