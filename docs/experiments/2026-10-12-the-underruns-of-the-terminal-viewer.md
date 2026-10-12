# The underruns of the terminal viewer

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal

## Objective

The viewer of the terminal (`spikes/terminal-av`) reported 0 to 4 underruns of its sound in a run of 15 seconds, from about the time it learned to join a room. What they were, before somebody listens to it.

## What was found

Two things, neither of them the room, the keys, a lock or the decoder.

- **Most were no underruns.** The output goes on asking for samples after the viewer has stopped taking packets, until it is closed, and every such call that found nothing was counted: one to four of them, for as long as the picture's thread took to end. Not counted any more (`Sound::end`).
- **One in six runs of 30 seconds had a real one**, 11.6 seconds in. A packet that was due up to 10 ms in the future was begun at once, so that packets join without a gap of a few samples; that held for the first packet after silence as well, and then every packet after it was played 10 ms early: the buffer of 50 ms was 40 for the rest of the run. The output asks 13.7 ms ahead of when a sample leaves the machine, about 10 ms at a time, and the packets came up to 21 ms later than the fastest, so 40 was not enough for the latest of them. A packet now joins the one before it early, and one that follows silence begins when it is due.

What was looked at and found sound: the callback of the output and the picture's thread take different locks; a packet is decoded (2.5 ms) outside the lock of the sound, on the task that reads the session; the packets came 2 to 9 ms later than the fastest at the median and 12 to 21 ms at most, at 640x360 and at 1920x1080 alike.

## Commands

```sh
SIZE=360 spikes/terminal-av/try.sh 30 ; SIZE=1080 spikes/terminal-av/try.sh 30
```

`AV_PLAYED` says when each underrun was (`underruns_at_s`), how far ahead the output asks (`output_ahead_ms`), and how much later than the fastest the Objects of each track came (`audio_came_over_least_ms`, `video_came_over_least_ms`).

## Observed results

Runs of 30 seconds, the picture to `/dev/null`, the sound at volume 0. **Nobody listened.**

| | 640x360, three runs | 1920x1080, three runs |
|---|---|---|
| With the end no longer counted, before the second correction | 0, 0, 0 | 0, 0, 1 |
| After both | 0, 0, 0 | 0, 0, 0 |
| A packet played after it was due, ms (median), after both | 1.0 to 1.4 | 1.8 to 2.4 |
| The same before (the two runs it was looked at in) | -8.8 and 8.1 | |

`try-room.sh` was run once after both: the sound had no underrun there either.

## What is left

- **The margin is small.** Of 50 ms, the output takes about 24 (13.7 ahead, and a call of about 10), and the latest packet of these runs 12 to 21: a path with more jitter than a loopback has will have underruns at 50 ms. `--buffer-ms` is the way out today; a buffer that follows how late the packets come, as the watching page has, is not built.
- On another machine or output device the 13.7 ms are another number.
- Nobody listened: a click where nothing is counted would not be seen here.

## Next action

A person listens.
