# The relay on cores of its own, and the writers of a session in one task

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, relay scale/load testing

## Objective

Two questions the [profile of today](2026-10-24-where-the-relay-spends-its-time-on-a-track-of-an-object-a-stream.md) left: whether the relay alone keeps up with 1,000 viewers of video and the sound-shaped track (an Object a stream) when the viewers do not share its cores, and whether running the writers of a session's subgroup streams in one task, where each stream has a task of its own today, takes a tenth or more off the relay's processor time. The writing of the sound stays an Object a stream (decided on 2026-10-13).

## Environment

As in the profile: podman's machine on the Mac (Linux 7.1.8, 5 cores, 8 GiB, shared with the user's idle containers), one container with the relay, the publishers and the viewers, the 320x180 video and the sound-shaped track, a viewer two sessions (one for each track). Built from main at `87dbae45` with the change below, in the container. No perf in these runs.

## Commands

```sh
spikes/fanout/linux/setup.sh
SPLIT=1 spikes/fanout/linux/run.sh split-1000 1000 small.h264 1          # the relay on cores 0-1, the rest on 2-4
spikes/fanout/linux/run.sh each-1000 1000 small.h264 1                   # a task a stream (as today)
RELAY_FLAGS='--writers session' spikes/fanout/linux/run.sh session-1000 1000 small.h264 1
```

- `SPLIT=1` pins the relay to two cores and the publishers and viewers to the other three with `taskset`. WHY not podman's `--cpuset-cpus`: rootless podman has no cpuset controller in that machine (`crun: controller cpuset is not available`). `top -H` reads the threads of the relay and of the viewers' processes for 5 seconds once the viewers have all begun.
- The change tried: `SendBudget::in_one_task` in `xmoq-moqt-io`, a budget whose writers run in one tokio task for the session, in a `FuturesUnordered` of `futures-util` 0.3.31, where `OutgoingSubgroup::spawn` otherwise spawns a task for each; the writer of each stream, with its queue and its signals, stays as it is. The relay took it with `--writers session` (`--writers each`, the default, as today), so that the two ran from one binary.

## Observed results

### The relay on two cores of its own

Video and sound, 1,000 viewers (2,000 sessions), a task a stream; three runs.

| Run | Relay, processor (mean / highest) | Relay, processor time of the run | Relay, memory (highest) | The machine's cores busy (mean / highest, of 500%) | Sessions that failed (video, sound) | Video: late at the worst 99th percentile |
|---|---|---|---|---|---|---|
| 1 | 68% / 124% | 19.4 s | 508 MB | 181% / 305% | 117, 42 | 7.9 s |
| 2 | 69% / 117% | 18.4 s | 408 MB | 189% / 303% | 136, 35 | 7.1 s |
| 3 | 74% / 127% | 21.9 s | 910 MB | 191% / 331% | 215, 88 | 10.7 s |

- **It did not keep up, and failed more than with the 5 cores shared**: 48 to 68 handshakes failed (none or 1 in the six runs below), up to 6,684 streams were reset with `TOO_FAR_BEHIND` (none below), the round trip of a session went up to 1.7 s, and a sound session had 250 of its 256 streams not done.
- **Nothing was at its limit of processor time**: the relay used 1.2 of its two cores at the highest, its two threads 70% to 83% and 30% to 60% in `top`; the viewers of the sound 3 threads of 20% to 50%, those of the video about 5% each; the machine was half idle. The relay's memory went from 444 MB to 850 MB in 2 seconds meanwhile: it held what it could not send.
- **So the 5 cores are not what the run of 1,000 lacks**, or not they alone. What it waits on was not found. On the way: the relay receives the packets of all its sessions on one UDP socket and one endpoint of quinn, where each viewer session has its own; a stream of the relay waits for the viewer to allow it one more (256 a session at most here).

### The writers of a session in one task

Video and sound, the 5 cores shared, as in the runs before.

| Viewers | A task a stream: relay's processor time of the run | One task a session |
|---|---|---|
| 500 | 10.3 s | 11.1 s |
| 1,000 | 21.5 s, 32.8 s, 25.5 s | 55.9 s, 33.2 s, 33.1 s |

- **One task a session did not take a tenth off; it took more**: at 500 by 8%, at 1,000 in each of three runs against three. At 500 both had every frame of the video in order.
- **At 1,000 a run either keeps up or falls behind, and the processor time with it**: the first run with a task a stream kept up (every frame of the video in order, all 500,000 Objects of the sound, the worst 53 ms late, 21.5 s); the other two fell behind (2.1 s and 5.4 s), as the run with perf this morning did (47.6 s). All three runs with one task a session fell behind (up to 7.7 s for the video and 11 s for the sound).
- The change was not kept: `crates/` is as on main.

## Limitations

- One machine, a virtual one; the viewers' processes are on the same machine also with `SPLIT=1`.
- Three runs at 1,000 and one at 500 for each way; at 1,000 the runs go one way or the other, and three is few for that.
- One way of putting the writers in one task was tried. A writer for a session that also does without the queue and the signals of each stream was not.

## Decision or next action

None. Where a run of 1,000 waits is open: the relay's one endpoint, the streams a session may have, or the viewers' processes.
