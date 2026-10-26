# Where the relay spends its time on a track of an Object a stream

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale/load testing

## Objective

The [Linux runs of today](2026-10-24-one-video-track-to-many-viewers-on-linux.md) found the track of the shape of the demo's sound (an Object on each stream, 50 a second) costly to the relay for its bytes, and 1,000 viewers of video and sound not kept up with. Where the relay's processor time goes there: the log of each stream, opening streams, allocation, locks, or something else; and what can be made lighter without changing what was decided (the sound's Objects each on a stream, decided on 2026-10-13).

## Environment

As in that record: podman's machine on the Mac (Linux 7.1.8, 5 cores, 8 GiB, shared with the user's idle containers), one container with the relay, the publishers and the viewers, the relay of main at `b2a8a7f1` built with `--release` and line tables (`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`). perf 6.12 of Debian 13 in the container samples the relay's user space (`cpu-clock:u`, 499 a second, stacks by DWARF) for 10 s once the viewers have all begun. The machine's `perf_event_paranoid` is 2, which keeps the kernel out of the samples: `sendmsg` and the rest of the kernel are not counted below.

## Commands

```sh
spikes/fanout/linux/setup.sh
PROFILE=1 spikes/fanout/linux/run.sh av-small-1000 1000 small.h264 1
```

With `PROFILE=1` the container runs without podman's seccomp profile and SELinux label (perf_event_open is refused with either), and `perf-groups.py` sorts the samples: by the tokio task whose future is on the stack, and by the relay's functions, names demangled by `rustfilt` 0.2.1 and matched without their generic arguments. The CPU of the viewers' two processes and the idle time of the machine (`/proc/stat`) are read every second beside the relay's.

## Observed results

One run each; the video is 320x180.

| | Video alone, 1,000 | Video and sound, 500 (1,000 sessions) | Video and sound, 1,000 (2,000 sessions) |
|---|---|---|---|
| Relay, processor while the viewers ran (mean / highest) | 31% / 55% | 70% / 96% | 155% / 197% |
| Relay, processor time of the whole run | 7.0 s | 10.3 s | 47.6 s |
| Relay, memory (highest) | 142 MB | 163 MB | 1,270 MB |
| The viewers' processes, video / sound (mean) | 36% / | 21% / 43% | 42% / 120% |
| The machine's 5 cores busy (mean / highest, of 500%) | 94% / 162% | | 384% / 467% |
| Everything in order | all 1,000 viewers, worst 32 ms | 500 and 360 viewers | 73 and 0 viewers; up to 7.8 s late |
| Samples of the relay in the 10 s | 1,189 | 2,541 | 6,268 |

Of the relay's samples, by the task they ran in:

| Task | Video alone, 1,000 | Video and sound, 500 | Video and sound, 1,000 |
|---|---|---|---|
| quinn's connection drivers (packets, encryption, acknowledgements) | 42.6% | 29.4% | 26.5% |
| The writers of subgroup streams (a task each) | 14.0% | 20.1% | 15.7% |
| The hub (one task for all sessions) | 7.7% | 11.9% | 24.2% |
| quinn's endpoint | 5.2% | 3.3% | 2.6% |
| tokio's blocking pool | 6.0% | 4.0% | 5.1% |
| The log's task | 1.2% | 2.6% | 0.7% |
| No task on the stack (the unwinding stopped, in the C library or the vDSO) | 20.2% | 25.7% | 23.1% |

Across tasks, of the relay's samples: the log's lines made (`describe` and `format`) 0.2% / 1.2% / 1.2% and queued (`Log::print`) 0.0% / 0.2% / 0.4%; `malloc` and `free` 3.4% / 4.8% / 3.5%; SipHash of the hash maps 2.4% / 3.9% / 4.5%; tokio's `Notify`, `mpsc` and `watch` 12.6% / 13.8% / 17.7%. In the hub at 1,000 with sound: handing an Object to a writer (`OutgoingSubgroup::send`) 5.1%, finishing a stream (`OutgoingSubgroup::finish`) 3.4%, the core's `handle_event` 3.4%, and `holds_nothing`, which the hub asks of every session after events that may change what a session holds, 1.8%.

- **The sound-shaped track costs the relay about 80 µs of processor time a stream**: 40.6 s more than the video alone at 1,000 viewers, for about 500,000 streams (1,000 viewers, 500 Objects each), and twice the sessions. The track carries 85 kbit/s a viewer.
- **That time is spread**: quinn's drivers, a task of each stream with its channels and its wake-ups, and the hub. No part of the relay's own code takes more than a few percent of it; no lock showed as a place where time is spent.
- **The log is not where it goes**: making and queueing the lines of each stream is 1.6% of the relay's samples at 1,000 with sound, and the log's task 0.7%, although about a million lines were dropped in that run.
- **The hub is not the limit**: at 1,000 with sound it is a quarter of the relay's samples, about 0.3 of a core of one task.
- **The machine is**: with 2,000 sessions its 5 cores were 77% busy at the mean and 93% at the highest, the relay taking 1.6 and the viewers' processes 1.6. Whether the relay alone keeps up with 1,000 viewers of both tracks on a machine of its own is not shown.

## What was not changed

Nothing of the relay was changed. The parts that could be made lighter without a decision come to a few percent, about what runs of the same kind differ by (the relay's processor time of a whole run of 500 with sound was 10.7 s, 10.1 s and 10.3 s in three runs today, two of them with perf):

- Not making a log line the queue has no room for: about 1.5% at 1,000 with sound.
- A hasher cheaper than SipHash for the keys the relay gives out itself, and asking `holds_nothing` only of the sessions an event is about: some of the 4.5% and the 1.8% above. Which keys a peer can choose decides whether SipHash can go.

What would take more off changes how the relay is built or what was decided, and was left to be decided:

- A writer task for each session, or each subscription, instead of each stream, without the four channels each stream has (`OutgoingSubgroup::spawn`): what of the writers' 16 to 20% and of tokio's wake-ups it saves is not measured.
- More than one Object of the sound on a stream: the decision of 2026-10-13.

## Limitations

- One run of each; the relay shares the 5 cores with the viewers.
- The kernel is not in the samples, and a fifth to a quarter of them have no task on their stack.
- `--no-inline`: a function the compiler put inline is counted in the one it was put in (`watch_idle` in the hub's loop, `describe` in `format`).

## Decision or next action

None. The options above are for the PM.
