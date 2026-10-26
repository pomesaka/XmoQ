# The audio run drops a piece that comes after the next

- Date: 2026-10-26
- Commit SHA: the commit that adds this record; the runs before the change were made on `main` at `1ee289d3`, the runs after it on its working copy on top of `49e30200`
- Roadmap item: none; the interop run (`interop/video.ts --video audio`)

## Objective

The pairing `own relay: audio: an own page encodes a tone as Opus and publishes it, and another decodes it` failed once in a full interop run of another working copy, whose change did not touch it: one piece was dropped as `late`, and piece 94 began 20 ms after the end of the one before it. Find why, and make the run not depend on how fast the machine is.

## What the failed run shows

From the log of that run (2026-10-26 11:07, sent by the session that ran it; the file of the pairing under `interop/.logs/matrix/` had been written over by a later run):

- The relay opened the 108 streams to the viewer in the order of their Groups: `OPEN_DATA session=1 … group=…` rises by 1 from one line to the next, with no gap and nothing out of order.
- The viewer's report: `"received":102,"dropped":{"late":1}`, and the publisher's `"written":107,"skipped":{"no-subscriber":8}`.
- The run failed with `piece 94 ({"timestamp":1792980475971999,...}) does not begin where the one before it ends, at 1792980475952000`.

## Why

- The publisher sends each chunk as a Group and a stream of its own (`createAudioTrackEncoder`), and nothing keeps the order of two streams.
- `playAudioTrack` decodes the Objects in the order of their times and drops one whose time is not after the last one decoded (`late`). An Object after a gap waits `holdForOrderMs` for the ones before it, and none waits where that is left out (`packages/media/src/core/in-order.ts`). The page of the interop run left it out; the demo's viewer passes the buffer of the sound less `ORDER_MARGIN_MS`.
- So a stream that reached the viewer before the one ahead of it went on at once, and the one ahead was dropped when it came. The relay opened them in order, so they changed places after it: in the relay's writing or in the viewer's Chromium. Which of the two was not found.
- The pieces after a dropped chunk carry their own times (`withTime` in `playAudioTrack`), so the piece after the gap begins 20 ms after the end of the one before it, as piece 94 did.

## Runs before the change

Each `mise run interop -- --base-port 37500 --only "own relay: audio: an own page encodes a tone"`, on macOS, arm64, 11 cores:

- 20 runs alone: 20 ended as expected.
- 20 runs with 11 busy loops of the shell beside them, one a core, under the lock of the full interop run so that no other run shared the machine: 20 ended as expected.

The failure was not made to happen again in 40 runs. What made it happen in the full run (many pages and QUIC connections at once, rather than busy cores) is a guess.

## The change

The page's `playAudio` takes `holdForOrderMs`, and the audio run passes `PLAY_TIMEOUT_MS` (10 s), as long as it waits for the report: a piece after a gap waits for the one before it until then. The limit stays, as every wait of a receiving side has one; a piece that never comes leaves the pieces short, which fails the run. `packages/media` is not changed. The other audio run (`audio-beyond-time`) passes nothing and waits for nothing, as before.

## Runs after the change

- 20 runs alone: 20 ended as expected.

Since the failure did not happen in 40 runs before the change, these runs cannot show that the change mends it. That it does is shown only by the full runs after it not failing in this way.

## Not looked at

- The first two Objects go on without waiting, whatever `holdForOrderMs` is: no step is known yet (`in-order.ts`). Two streams that change places there still drop one.
- The relay's 108 `OPEN_DATA` lines against the publisher's 107 written Objects: whether that is a difference of how each counts or one Object more was not looked at.
- Where between the relay and the page the two streams changed places.
