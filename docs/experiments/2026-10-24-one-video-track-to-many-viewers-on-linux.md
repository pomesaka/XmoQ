# One video track to many viewers on Linux, and with a track of the shape of sound

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, relay scale/load testing (Linux); Phase 3, the terminal viewer on Linux

## Objective

The runs of the [record of 2026-10-11](2026-10-11-one-video-track-to-many-viewers.md) on Linux: what the relay takes of a machine to send one video track to up to 1,000 viewers of the Rust client, how late the frames come, and what is lost. Then the same with a second track of the shape of the demo's sound, which has one Object on each stream (50 streams a second). And `xmoq-watch` once on Linux.

## Environment

- The Mac of these records (Apple M3 Pro), with podman 6.1.1's machine running: Linux 7.1.8 (Fedora 44, aarch64) under libkrun, 5 cores and 8 GiB, which other containers of the user's share (idle, about 3.6 GB of its memory in use before the runs). The Mac itself had little memory free (the machine's process `krunkit` held 15 GB).
- One container (Debian 13, `spikes/fanout/linux/Containerfile`) holds the relay, the publishers and the viewers, on loopback, with 3.5 GB at most. `xmoq-relay`, `xmoq-moqt-client` and `xmoq-watch` of main at `80d81c96` (the commit before this one), built in it with `--release` and Rust 1.99.0.
- The videos as on 2026-10-11 (ffmpeg 8.1, `testsrc2`, `h264_videotoolbox`, Baseline, 25 frames a second, a key frame a second, 100 frames): 320x180 at 527 kbit/s and 1280x720 at 2,540 kbit/s as sent. The track of the shape of sound: 64x64 frames, 50 a second, a key frame every 10, 85 kbit/s as sent (about 210 bytes an Object), sent with `--video-frame-ms 20 --video-streams object`: an Object on its own stream every 20 ms, as the demo's page sends its Opus.

## Commands

```sh
spikes/fanout/linux/setup.sh                                  # image, binaries, media, certificate, mlmpub
spikes/fanout/linux/run.sh small-500 500 small.h264           # video alone
spikes/fanout/linux/run.sh av-small-500 500 small.h264 1      # video and the sound-shaped track
spikes/fanout/linux/watch-run.sh                              # xmoq-watch
```

These runs used the image `localhost/iori-xmoq-linux` and volumes of that name (`NAME=iori-xmoq`), made for them and removed after. The runs of the tables ran the same scripts from a scratch directory before they were put under `spikes/fanout/linux/`; one (100 viewers with both tracks) was run again with these, within the spread of the table (19% / 26%, nothing lost or out of order), and `watch.sh` ran from there.

- As on 2026-10-11: the publisher is the Rust client with `--video-in`; the viewers are N sessions of one process, 10 ms apart, each taking 250 frames (10 s) and saying how late each frame came (`VIDEO_STATS`). The relay runs with `--max-sessions 2100`.
- **With the sound-shaped track, a viewer is two sessions**: a second process of N sessions subscribes to it, 500 Objects each. The Rust client takes one track a session, so 1,000 viewers are 2,000 sessions here, where a page or `xmoq-watch` has both tracks on one.
- The relay's processor time is read from `/proc/<pid>/stat` every second, the difference of `utime` and `stime`: 100% is one core. WHY not `ps` as on the Mac: Linux's `ps` gives `%cpu` over the life of the process (a first series with it said 15% for 500 viewers; it was taken anew). Memory: `VmRSS` of the relay, and `free` of the machine.
- The relay writes a `STATS` line for every session every second (`--stats-every-ms 1000`).

## Observed results

One run each. Lateness: median of the viewers' medians / worst 99th percentile / highest, in ms. "In order" is the Objects a viewer took in the order its writer takes them; "out of order" is those that came, but after an Object of a later Group (the writer of the client does not go back, `AnnexBWriter` in `crates/moqt-client/src/video.rs`), which were not lost.

### Video alone

| Video | Viewers | Relay, processor (mean / highest) | Relay, memory (highest) | Lateness | Out of order | Viewers that failed |
|---|---|---|---|---|---|---|
| 320x180, 527 kbit/s | 10 | 2% / 4% | 8 MB | 0.5 / 2.9 / 9.4 | 0 | 0 |
| | 100 | 8% / 15% | 22 MB | 1.2 / 5.3 / 14.0 | 0 | 0 |
| | 250 | 17% / 28% | 45 MB | 2.3 / 17.6 / 21.5 | 0 | 0 |
| | 500 | 24% / 32% | 85 MB | 3.6 / 19.6 / 29.7 | 0 | 0 |
| | 1,000 | 35% / 54% | 147 MB | 5.3 / 218.0 / 218.1 | 0 | 0 |
| 1280x720, 2,540 kbit/s | 10 | 2% / 5% | 9 MB | 0.6 / 2.8 / 5.1 | 0 | 0 |
| | 100 | 12% / 20% | 29 MB | 1.8 / 11.3 / 14.2 | 0 | 0 |
| | 250 | 23% / 33% | 63 MB | 3.3 / 15.4 / 22.0 | 0 | 0 |
| | 500 | 34% / 76% | 123 MB | 5.6 / 121.3 / 164.7 | 0 | 0 |
| | 1,000 | 34% / 71% | 214 MB | 7.0 / 136.7 / 216.2 | 0 | 0 |

- **Every viewer got every frame in order in every run, 1,000 viewers of the large video too**, the worst 216 ms late. On the Mac the large video fell behind from 500 (1.4 s at 500, 5 s and 29% of the frames skipped at 1,000), and with `fast-apple-datapath` 500 came whole at 117 ms worst.
- The relay took a fifth of what it took on the Mac at 11 cores: 0.7 core at the highest for 1,000 viewers of the large video (2.5 Gbit/s sent), where the Mac took 1.8 cores at the mean and 3.1 at the highest. On Linux `quinn-udp` sends up to 64 packets with one `sendmsg` (the record of 2026-10-11 read this in its source; it is not traced here).
- No packet was lost and no congestion event was counted in the `STATS` lines; no stream was reset but those the viewers stopped (`CANCELLED`); nothing waited to be written for more than a moment (at most one Object of 12 KB). The relay's log dropped about 5,000 lines in each run of 1,000 (its queue is bounded, `LOG_DROPPED`), so some `STATS` lines of those runs are missing.

### Video and the sound-shaped track (two sessions a viewer)

| Video | Viewers (sessions) | Relay, processor (mean / highest) | Relay, memory (highest) | Video: lateness / out of order | Sound: lateness / out of order of all Objects | Viewers that failed (video, sound) |
|---|---|---|---|---|---|---|
| 320x180 | 100 (200) | 22% / 28% | 36 MB | 0.9 / 5.5 / 9.1; 0 | 1.0 / 4.3 / 8.6; 0 | 0, 0 |
| | 250 (500) | 43% / 54% | 82 MB | 1.9 / 7.2 / 31.2; 0 | 2.0 / 7.4 / 11.5; 0 | 0, 0 |
| | 500 (1,000) | 59% / 103% | 164 MB | 4.0 / 34.5 / 71.6; 0 | 4.1 / 27.0 / 46.0; 85 of 250,000 | 0, 0 |
| | 1,000 (2,000) | 131% / 201% | 931 MB | 24.9 / 3,261 / 3,261; 94,541 of 249,750 | 17.4 / 3,426 / 3,449; 201,945 of 499,500 | 1, 1 |
| 1280x720 | 250 (500) | 51% / 88% | 95 MB | 3.2 / 27.7 / 38.5; 0 | 2.3 / 22.6 / 36.3; 741 of 125,000 | 0, 0 |
| | 500 (1,000) | 59% / 114% | 195 MB | 6.1 / 156.0 / 236.8; 0 | 4.5 / 27.7 / 44.1; 6,808 of 250,000 | 0, 0 |
| | 1,000 (2,000) | 135% / 202% | 1,190 MB | 34.0 / 9,444 / 9,444; 97,997 of 248,546 | 16.9 / 6,287 / 6,310; 239,629 of 497,500 | 10, 5 |

- **Up to 500 viewers (1,000 sessions) every Object came**, of both tracks and either video: the video in order and within 237 ms, the sound within 46 ms of what was taken in order. A part of the sound came out of order (up to 2.7% with the large video), which one stream an Object allows; a player of sound puts an Object at its time and would not lose these, the tool here counts them apart.
- **With 1,000 viewers (2,000 sessions) it did not keep up**: Objects came up to 3.3 s late with the small video and 9.4 s with the large one, and with the large one 18,740 packets were lost, 284 congestion events counted, a round trip of 773 ms seen, 2 handshakes failed, 15 sessions failed (no session within 10 s, or the connection lost), the relay reset 514 streams with `TOO_FAR_BEHIND` and ended 2 subscriptions for a send queue that was full (`PUBLISH_DONE` 0x5). The relay held 0.9 to 1.2 GB.
- **The sound-shaped track costs more than its bytes**: with 500 viewers of the small video the relay took 59% at the mean against 24% for the video alone, for 85 kbit/s more a viewer; at 100 viewers 22% against 8%. That is 25,000 streams opened and closed a second at 500, each with a line of the log: the log dropped 313,000 lines in that run and about a million in each run of 1,000.
- Which of the relay and the viewers' two processes was too slow at 1,000 is not told apart: they share the 5 cores, and the relay took 2 of them at the highest.
- **The relay's `STATS` lines are incomplete in these runs**: the log dropped them with the rest (1,853 `STATS` lines in the run of 1,000 small, where 2,000 sessions of about 10 seconds would have written some 20,000).

## xmoq-watch on Linux

In the same kind of container, against `mlmpub` of moqlivemock v0.16.1 (built for linux/arm64 from the commit `scripts/ci/interop.sh` names), as `interop/watch-rust.ts --watch from-mlmpub` does, 8 seconds each (`spikes/fanout/linux/watch.sh`):

| How | Ended | Frames drawn (came) | Packets of sound | Underruns | Picture after its time, ms (median / highest) |
|---|---|---|---|---|---|
| No terminal, no sound output (`--out /dev/null --sound off`) | exit 0 | 189 (190), 25.1 a second | 384 | 0 | 3.6 / 23.2 |
| A pseudo-terminal of `script` as its terminal (`TERM=xterm-kitty`, `--fit 80x24`, `--sound off`) | exit 0 | 192 (193), 25.0 a second | 391 | 0 | 1.7 / 5.8 |
| The sound on | exit 1 at once: `The requested audio device is not available` | | | | |

- It found the video and the audio track among the 13 of the catalog, decoded 1280x720 in 2.6 ms a frame, and put the terminal back (exit 0) in both runs that played.
- The sound output did not open: the container has ALSA's library and no device (`Unknown PCM default`). Nobody saw the picture or heard the sound; the pseudo-terminal's bytes (27 KB) were not read.
- `mise run interop` has run the first of these on ubuntu-24.04 in CI for every pull request that touches its paths (on 2026-10-21 on main: 237 frames and 399 packets of sound of a page, 174 and 355 of `mlmpub`). The documents said it had only been built on Linux.

## Limitations and blockers

- One machine, and a virtual one: the viewers, the publishers and the relay share 5 cores of a VM; loopback has no loss and no delay but what the receivers drop. The numbers are of the relay's own work there, not of a network.
- One run each, 10 seconds a viewer; with the sound-shaped track a viewer is two sessions, unlike a real viewer.
- The track of the shape of sound is H.264 frames of the size of Opus packets, not Opus.
- The relay's log drops lines under load, `STATS` lines among them.
- The media files were made again with the same commands after the runs; whether the bytes are the same was not checked.

## Decision or next action

None. More than one machine and real links are still open.
