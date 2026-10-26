# Why a run of 1,000 viewers with sound falls behind with cores to spare: what it is not

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale/load testing

## Objective

1,000 viewers of video and the sound-shaped track (an Object a stream, a viewer two sessions) on the Linux VM either keep up or fall behind by seconds, and fall behind also when the relay has cores to spare ([record](2026-10-24-the-relay-alone-and-the-writers-of-a-session-in-one-task.md)). Three places were named to look at: the relay's one UDP socket and quinn endpoint, the streams a viewer allows the relay, and the viewers' processes. The work was given a time limit; this records what was tried. The cause was not found.

## Environment

As in that record: podman's machine on the Mac (Linux 7.1.8, 5 cores, 8 GiB), one container, the relay of main at `87dbae45` (before the GOAWAY of a request stream came to main), the 320x180 video. The Mac had about 44 MB of memory free and the machine's process (`krunkit`) held 15 GB.

## Commands

```sh
VIEWER_PROCS=2 spikes/fanout/linux/run.sh p2-1000 1000 small.h264 1    # the viewers of each track in two processes
spikes/fanout/linux/run.sh stall-1000 1000 small.h264 1                # stalls.py runs beside, as in every run now
```

- `stalls.py` sleeps 5 ms again and again inside the container and writes each second how much later than asked it woke: a VM whose cores its host takes away shows as long oversleeps.
- For one run the relay was built with a line, once a second, of how late its hub came to its timer and how many envelopes waited for it (`HUB_STATS`, to stderr so that the log does not drop it). It was taken out again; `crates/` is as on main.
- The `STATS` lines of earlier runs of today were read again for the round trip, the congestion window, what the transport held (`handed_bytes` against `sent_bytes`), and the streams waiting to be opened.

## Observed results

| What was looked at | Run | Seen |
|---|---|---|
| The viewers' processes | 2 processes a track (`p2-1000`) | Fell behind as with one: the video up to 5.4 s and the sound 6.6 s late, the relay 34.3 s of processor time. No thread of the viewers was busy all the time in any run (`top -H`: 5% to 50% each). |
| Streams a viewer allows | `STATS` of the runs with the 5 cores shared | Streams waiting to be opened: at most 1 in the run that kept up, 4 and 3 in two that fell behind, 124 in `p2-1000`; a session's streams not done, 99th percentile 20 to 63. The Rust client lets the relay open 64 streams (and 3 of HTTP/3) at once. Only with the relay pinned to two cores were 255 streams waiting. |
| The UDP socket and quinn | the same | The round trip of a session, 99th percentile 9 ms and 15 ms in runs that fell behind (4 ms in the one that kept up, highest 663 ms in `p2-1000`); no packet lost, no congestion event; the congestion window of most sessions at its first 12,000 bytes; what quinn held of a session (`handed_bytes` - `sent_bytes`) 0.8 MB at the most. The seconds of delay are not in the transport of the relay's sessions. |
| The VM | `stall-1000` | The longest oversleep in a second: 10.7 ms; the VM did not stop for long. |
| The hub | the run with `HUB_STATS` | Late to its timer by 0 to 4 ms every second, with no envelope waiting, in a run whose video came up to 1.7 s late at the worst 99th percentile. The hub is not behind. |

- **The delay is common to the viewers**: in the runs that fell behind, 853 and 852 of 1,000 viewers had a 99th percentile over 0.5 s (the median of them 0.9 s and 2.0 s); in the run that kept up, 10 viewers were over 100 ms and none over 0.5 s. Something they all pass through holds the Objects.
- That is not the hub, nor the transport of the relay's sessions, nor the viewers' processes alone, nor the VM stopping. What remains of the way is the publishers: their processes, and their connections into the relay, of which no line says anything. They were not measured.
- **What decides whether a run keeps up was not found.** The relay's processor time is larger in a run that falls behind (21.5 s in the one that kept up, 24 to 56 s in those that did not), which follows from falling behind and does not tell why.

## Not done

- A viewer that allows more streams (a flag of the Rust client): the streams were not where the waiting was, with the cores shared.
- quinn's own counters for a connection beyond what `STATS` has.
- The publishers' side: how late an Object leaves its publisher and arrives at the relay. The relay does not read LOC, and its log has no time; a line of the publisher's client with the time each Object was written, or a relay line of when the Objects of a track arrive, would tell.

## Decision or next action

None. The next place to measure is the publishers' side.
