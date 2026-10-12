# The terminal viewer asks for a delivery timeout

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal; Phase 2, quality under a narrow link

## Objective

The relay acts on the `SUBGROUP_DELIVERY_TIMEOUT` of a SUBSCRIBE now, and the watching page can ask for one on its video. Whether the viewer of the terminal, `xmoq-watch`, can do the same, what it gets for it behind a link with less room than the video, and whether it goes on after a Group whose end the relay gave up.

## What was built

`xmoq-watch --video-timeout-ms N`: the SUBSCRIBE of the video track carries `SUBGROUP_DELIVERY_TIMEOUT` (§10.2.3 of draft-ietf-moq-transport-18) with N milliseconds. Not asked without the flag; 0 is not asking (§8: "a value of 0 means that there is no timeout set"). The sound's SUBSCRIBE and the catalog's carry none. The parameter is encoded by `xmoq-moqt`, as for every other client of ours.

## Environment

One machine, macOS, loopback. The demo's relay with no flag of its own, its publishing page in headless Chromium with `?bitrate=2500000&picture=busy` (640x360, 30 frames a second, a picture that fills its 2.5 Mbit/s), and between the relay and the viewer `spikes/lossy-link/proxy.mjs`: 2,000 kbit/s, a queue of 100 ms, 50 ms each way, no random loss. `xmoq-watch` with `--release`, `--sound off`, the picture to `/dev/null`.

## Commands

```sh
crates/watch/scripts/try-narrow-link.sh 25 [--video-timeout-ms 100] [--buffer-ms 300]
```

## Observed results

Two runs of each, about 24 seconds of watching; about 535 frames and 1,200 packets of sound came in each. "Late" is how much longer than the fastest Object an Object took to come.

| Timeout of the video | Buffer | Frames drawn | Video late, ms (median / greatest) | Streams the relay reset | Sound played, s | Underruns | Sound late, ms (median) |
|---|---|---|---|---|---|---|---|
| none | 50 ms | 146, 152 (6 a second) | 2,331 / 9,949 and 2,318 / 9,918 | 0 | 7.9, 8.8 | 1,693, 1,644 | 78, 74 |
| 100 ms | 50 ms | 532, 533 (22 a second) | 271 / 480 and 276 / 539 | 23 | 21.0, 14.3 | 356, 988 | 47, 65 |
| none | 300 ms | 149, 149 | 2,345 / 9,841 and 2,306 / 9,745 | 0 | 21.5, 22.0 | 0 | 97, 64 |
| 100 ms | 300 ms | 532, 528 | 269 / 523 and 274 / 502 | 23 | 23.5, 23.4 | 0 | 66, 64 |

- **With the timeout the picture stops falling behind**: a quarter of a second late and no more, where without it the lateness grows to 10 seconds within the run. Three and a half times the frames are drawn.
- **The viewer goes on after a Group that was cut**: 23 streams were reset in a run, no frame after one was left out for it, and the picture went on with the next Group. Without the timeout most frames are left out by the viewer itself: when the key frame of a newer Group has come, the frames of the older one that come after it are not drawn (386 to 392 of about 538).
- **The sound is not kept by the timeout alone, with the viewer's buffer of 50 ms**: on this link a packet of sound comes 47 to 97 ms later than the fastest at the median, which is more than the buffer, and a third to a half of the sound was not played. With 300 ms none of it was late enough for an underrun, with or without the timeout. The watching page widens its buffer as packets come late; the viewer does not ([its README](../../crates/watch/README.md)).
- Without the timeout 3 or 4 Objects a run came more than 10 seconds late and were left out as not of the clock.

## Not measured

- Sound that was heard, a picture that was seen.
- Other timeouts than 100 ms, other links, several viewers, the relay's own flags together with the timeout.
- A timeout on the sound.

## Next action

None from this record. That the viewer's buffer does not follow the lateness is the open part its README names.
