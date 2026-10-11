# One video track to many viewers through the own relay

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay (how far one relay carries one track)

## Objective

Numbers for the relay's fan-out: what it takes of a machine to send one video track to N subscribers, how late the frames come, and where it stops.

## Environment

One machine, macOS, 11 cores, loopback. The relay, the publisher and the viewers all run on it, so the viewers take processor time the relay could have had. `xmoq-relay` and `xmoq-moqt-client` of the working copy this record is part of, built with `--release`.

Two videos made with ffmpeg 8.1 (`testsrc2`, `h264_videotoolbox`, Baseline, 25 frames a second, a key frame a second, 100 frames): 320x180 at 513 kbit/s, and 1280x720 at 2.5 Mbit/s.

## Commands

```sh
spikes/fanout/run.sh <viewers> <file.h264> 250
```

- The publisher is the Rust client with `--video-in <file>`, new: for every line on its stdin it sends the next Group of the file on one stream, a frame every 40 ms, each with the wall-clock time as its Timestamp of LOC, and begins again after the last.
- The viewers are N sessions in one process (`--viewers N`, new), 10 ms apart, each with its own connection and SUBSCRIBE. Each takes 250 Objects and says how many microseconds lay between the Timestamp of a frame and the clock when it had the frame (`VIDEO_STATS`).
- The script reads `ps -o %cpu,rss` of the relay once a second while the viewers run. 100% is one core.

## Observed results

One run each.

| Video | Viewers | Relay, processor (mean / highest) | Relay, memory (highest) | Lateness of a frame: median of the viewers' medians / worst 99th percentile / highest | Frames skipped after a viewer's first key frame | Viewers that failed |
|---|---|---|---|---|---|---|
| 320x180, 513 kbit/s | 1 | 1% / 2% | 9 MB | 1.0 / 1.8 / 1.9 ms | 0 | 0 |
| | 10 | 4% / 6% | 12 MB | 1.2 / 2.9 / 3.2 ms | 0 | 0 |
| | 100 | 39% / 57% | 28 MB | 3.2 / 15.9 / 18.4 ms | 0 | 0 |
| | 250 | 79% / 110% | 53 MB | 5.4 / 21.2 / 26.1 ms | 0 | 0 |
| | 500 | 66% / 87% | 52 MB | 5.4 / 21.7 / 24.8 ms | 0 | 245 |
| 1280x720, 2.5 Mbit/s | 1 | 2% / 3% | 12 MB | 0.8 / 3.0 / 3.7 ms | 0 | 0 |
| | 10 | 18% / 24% | 17 MB | 2.1 / 7.2 / 8.1 ms | 0 | 0 |
| | 100 | 87% / 110% | 47 MB | 7.4 / 27.5 / 30.3 ms | 0 | 0 |
| | 250 | 165% / 251% | 117 MB | 13.0 / 92.2 / 106.6 ms | 0 | 0 |
| | 500 | 152% / 223% | 109 MB | 11.6 / 72.6 / 93.4 ms | 0 | 245 |

- **At 500, 255 viewers were served and 245 were refused**: the relay takes 256 sessions (`max_sessions`, a limit of its own), and the publisher is one. Its log has `SESSION_REFUSED ... too many sessions`; the viewers saw the session closed with that reason, or the connection closed. The rows for 500 are 255 viewers.
- **Up to 250 every viewer got every frame**: none skipped after its first key frame, none cut off by the relay, in either video.
- A viewer that comes in the middle of a Group gets the rest of that Group, which it cannot decode, and waits for the next key frame: 12 frames on average, half a second.
- The relay's processor time grows with the viewers and with the bit rate, about 0.35% of a core for a viewer of the small video and 0.7% to 0.9% for one of the large. It used more than one core from 250 viewers of the large one.
- Lateness stays within a frame (40 ms) up to 100 viewers of either video and 250 of the small one; with 250 of the large one the worst viewer's 99th percentile is two frames.

## Limitations and blockers

- One machine: the viewers and the relay share its cores, and loopback has no loss and no delay. The numbers say what the relay's own work costs here, not what a network does.
- One run each, 10 seconds of video a viewer.
- The lateness is from the publisher's clock when it wrote the frame to the viewer's when it had decoded the Object, on one machine.
- Memory and processor time are of the whole relay process, read once a second.
- More than 255 viewers were not measured: the limit of sessions was left as it is.
- The videos are a test pattern, and of constant bit rate only as far as the encoder kept to it.

## Decision or next action

None. The limit of 256 sessions is the first thing a larger run meets.
