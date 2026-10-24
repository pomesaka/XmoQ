# Ten rooms of a hundred viewers

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, relay scale/load testing; the goal of Q41

## Objective

The goal decided on 2026-10-24 (Q41): a delay of 500 ms at most, 100 viewers a room, 10 rooms open at once. Whether one relay meets it on the Linux VM of today's runs, with each room a publisher of video and sound: the delay at the 99th percentile, what is lost, the relay's processor and memory. Where it does not, how many rooms and viewers it does.

## Environment

As in the [Linux runs of today](2026-10-24-one-video-track-to-many-viewers-on-linux.md): podman's machine on the Mac (Linux 7.1.8, 5 cores, 8 GiB, shared with the user's idle containers), one container with the relay, all publishers and all viewers, sharing the 5 cores; the relay of main at `34bb2709`, with `--release`. A room is a namespace of its own (`xmoq/fanout/<r>`) with a publisher of the 320x180 video (527 kbit/s, a Group a second on a stream) and one of the sound-shaped track (85 kbit/s, an Object every 20 ms on a stream of its own), and its viewers: a session for the video and one for the sound each, in two processes of the Rust client a room. A viewer takes 250 frames (10 s).

**The delay measured** is from the Timestamp a publisher writes into an Object to the moment a viewer has it; the viewers of the Rust client show nothing. A page shows a frame its buffer later (150 ms by default since 2026-10-21), so the goal of 500 ms from made to shown leaves about 350 ms for this one.

## Commands

```sh
spikes/fanout/linux/setup.sh
ROOMS=10 spikes/fanout/linux/run.sh rooms10x100 100 small.h264 1    # 10 rooms of 100 viewers
```

`ROOMS` is new in `fanout.sh`; the summary now also counts the viewers whose 99th percentile is over 500 ms.

## Observed results

One run each, three of the goal's shape. Delay: the median of the viewers' medians / the worst viewer's 99th percentile, and the viewers whose 99th percentile is over 500 ms.

| Rooms x viewers (viewer sessions) | Relay, processor (mean / highest) | Relay, processor time | Relay, memory | The 5 cores busy (mean) | Video: delay; over 500 ms | Sound: worst 99th percentile; over 500 ms |
|---|---|---|---|---|---|---|
| **10 x 100 (2,000)**, run 1 | 176% / 186% | 35.6 s | 1,494 MB | 444% | 2,403 ms / 7,979 ms; 990 | 8,941 ms; 696 |
| 10 x 100, run 2 | 174% / 185% | 49.4 s | 1,373 MB | 434% | 83 ms / 11,247 ms; 995 | 9,505 ms; 994 |
| 10 x 100, run 3 | 174% / 194% | 41.6 s | 1,522 MB | 443% | 1,227 ms / 9,029 ms; 984 | 8,780 ms; 980 |
| 8 x 100 (1,600) | 174% / 191% | 28.9 s | 1,000 MB | 411% | 47 ms / 7,928 ms; 800 | 3,544 ms; 800 |
| 10 x 75 (1,500) | 177% / 190% | 34.6 s | 1,040 MB | 432% | 816 ms / 5,712 ms; 750 | 5,080 ms; 750 |
| 10 x 60 (1,200) | 107% / 172% | 14.2 s | 268 MB | 287% | 2.2 ms / 964 ms; 17 | 245 ms; 0 |
| 5 x 100 (1,000) | 87% / 103% | 8.8 s | 145 MB | 231% | 2.2 ms / 17.2 ms; 0 | 17.2 ms; 0 |
| 10 x 50 (1,000) | 88% / 93% | 9.2 s | 139 MB | 235% | 0.9 ms / 9.4 ms; 0 | 7.6 ms; 0 |
| 10 x 25 (500) | 54% / 61% | 5.8 s | 74 MB | 169% | 0.5 ms / 3.5 ms; 0 | 5.4 ms; 0 |

- **The goal was not met: 10 rooms of 100 fell behind by seconds in all three runs**, nearly every viewer over 500 ms. In two of them every viewer of the video (and in one every viewer of the sound) failed to get its Objects within its time (25 s); 3 to 17 handshakes failed. No packet was lost.
- **500 viewers in all (1,000 sessions) kept up**, as 5 rooms of 100 and as 10 rooms of 50: the worst viewer within 17 ms. 600 (10 rooms of 60) was at the edge: 17 viewers of the video over 500 ms, the sound within 245 ms. From 750 on every run fell behind.
- Ten publishers of a tenth each did not do better than one publisher of all: one room of 1,000 kept up in one of three runs today, ten of 100 in none.
- The relay used under 2 of the 5 cores where it fell behind, and the machine was 82% to 89% busy at the mean: the viewers' twenty processes share it. That the relay alone would keep up on a machine of its own is not shown, and the record before it found the relay falling behind with cores to spare ([record](2026-10-24-why-a-run-of-1000-viewers-with-sound-falls-behind-with-cores-to-spare.md)).

## Limitations

- One machine, a virtual one; publishers, viewers and relay on the same 5 cores and loopback.
- One run each but for the goal's shape; the runs near the edge vary.
- A viewer's delay here is to its arriving, not to its being shown.
- The sound is H.264 frames of the size of Opus packets, not Opus.

## Decision or next action

None. The goal is met on this machine up to about 500 viewers in all, not 1,000. More than one machine (the viewers elsewhere) is what would tell what the relay alone does.
