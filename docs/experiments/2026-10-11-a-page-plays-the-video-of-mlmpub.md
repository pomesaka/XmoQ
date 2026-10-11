# A page plays the H.264 video of moqlivemock's publisher

- Date: 2026-10-11
- Commit SHA: the commit that adds `playVideoTrack` to `@xmoq/media` and the run to the interop matrix, together with this record. The runs were made on its working copy, on `main` at `20452147` with the receiver and the sender of `@xmoq/media` below it
- Roadmap item: Phase 1, "Decode and render video". Decoded, from a publisher that is not ours; nothing is drawn, and the item stays open

## Objective

The receiver of `@xmoq/media` had been run against tests and two captured payloads. This connects it: a subscription of `@xmoq/moqt` on one side, the H.264 decoder of the browser on the other. The run is to show that a page decodes the video of `mlmpub`, a publisher that is not ours, frame after frame.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Publisher | `mlmpub` of `Eyevinn/moqlivemock` v0.16.1 (commit `91d903e9`), unchanged; the track `video_400kbps_avc` of `mlm/msf/clear` |
| Drafts | draft-ietf-moq-transport-18; draft-ietf-moq-msf-01; draft-ietf-moq-loc-04 |

## Commands

```sh
mise run check
mise run interop -- --base-port 45300
(cd interop && bun video.ts --video from-mlmpub)   # the one run, alone
```

## Observed results

What has to be seen was written at the top of `interop/video.ts` before the first run, which came out as written.

| Expected | Seen |
|---|---|
| The catalog has `video_400kbps_avc` with `avc3.4D401F`, 1280x720, 25 frames a second; the decoder is configured with that codec string, for Annex B, without a description | As expected |
| At least 50 frames are decoded | 50 when the report was taken, of 50 given to the decoder |
| The first frame is the first of a Group: its time is a whole number of seconds | 1791698325000000 |
| Every frame is 1280x720, and 40,000 microseconds after the one before | As expected, for all 50 |
| The decoder did not fail | 0 failures |
| Nothing was dropped but what came before the first key frame | Nothing was dropped at all: `mlmpub` begins a subscription at the start of a Group |

- `mise run interop`: 105 of 105 pairings, the 104 from before and this one.
- Temporary changes, each taken back, with the run alone:

| Change | The run |
|---|---|
| The payload is given to the decoder as it came, with lengths | fails: `decode` throws, the decoder is made anew, and after three failures the playing ends with no frame; the other Objects are `late` |
| No frame is marked as a key frame | fails in the same way |
| The time of every frame is 0 | fails: a frame 0 microseconds after the one before |
| Every second frame is kept from the decoder | fails: a frame 80,000 microseconds after the one before. The decoder gave frames all the same |
| A Group begins with whatever Object of it is there, not with Object 0 | **passes**: this publisher never has a viewer come into the middle of a Group |

### After the review of `playVideoTrack`

- **A decoder whose `decode()` threw was left open.** Seen in Chromium, with the payload given as it came: the decoder's state after the throw is `configured`, three times; it is `closed` after the change, which closes a decoder that failed before the next one is made.
- The decoder is made before the SUBSCRIBE, and the loop that reads the subscription cancels it however the loop ends; before, a failure in that loop left the subscription open and `ended` unsettled.
- **A browser without a decoder for the codec is told apart from one that decodes nothing**: `playVideoTrack` rejects with NotSupportedError before it subscribes, and the driver asks `VideoDecoder.isConfigSupported` first and says so. Both were seen with temporary changes that made the answer "not supported": the run fails with "this browser has no decoder for avc3.4D401F", and not with a count of frames.
- The page reports after 10 seconds at the latest, with what it has; a run with too few frames says how the playing ended and what the decoder said last.
- The run as it is passes as before: 50 frames.

## Limitations and blockers

- **The pictures were not looked at**: that a frame of the right size comes out at the right time is all that is checked. A decoder that was given every second frame still gave frames.
- **A viewer in the middle of a Group, a lost frame, a decoder that recovers and one that is too slow were not run.** The first two are in the tables of the receiver; the last two are the code of `playVideoTrack`, which has no test without a browser.
- No relay: `mlmpub` is a server, and the page connects to it. Video through the own relay waits for a sender.
- 50 frames, two seconds, one track, loopback, the headless build. No hardware decoder was asked for or looked at.
- The connection of the page takes Objects of up to 16 MiB; the limit a viewer is to set was not set.

## Open questions raised

- The numbers of `playVideoTrack`, 8 chunks waiting in the decoder and 3 failures in a row, are not from a measurement.
