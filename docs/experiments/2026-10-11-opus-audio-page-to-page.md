# Opus audio from a page to a page, through the relay

- Date: 2026-10-11
- Commit SHA: the commit that adds the audio parts of `@xmoq/media`, together with this record; the runs were made on its working copy
- Roadmap item: Phase 2, "Build the audio pipeline". The first step of it: both ends, without a buffer and without sync

## Objective

Encode audio in a page with `AudioEncoder` (Opus), publish it as a LOC track of its own, and decode it in another page with `AudioDecoder`, through the own relay.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay | own |
| Drafts | draft-ietf-moq-transport-18, draft-ietf-moq-loc-04 |

## Commands

```sh
mise run check
mise run interop -- --base-port 45300
(cd interop && bun video.ts --video audio)
mise run demo        # and the two pages, opened by a script, headless
```

## Observed results

`--video audio`: a page makes 20 ms of a 440 Hz tone, half of full scale, every 20 ms as `AudioData`, encodes it as `{ codec: "opus", sampleRate: 48000, numberOfChannels: 1 }` and publishes each chunk as a Group with one Object on a stream of its own; another page decodes with the same three values and no `description`. What had to be seen was written at the top of `interop/video.ts` before the first run.

- 100 pieces decoded, each 960 frames at 48,000 Hz, mono; the largest sample 0.50 of 1; nothing dropped; no encoder or decoder failed; a chunk is 130 bytes on average. 4 runs of 4.
- **One expectation was wrong**: "each piece begins exactly where the one before it ends". The time of piece 13 was 1 microsecond before that, though the Objects carry whole multiples of 20,000 after the first: the decoder counts the times of what it gives by itself. The run takes 2 microseconds, and saw 1 at most.
- **The encoder gave a `description` of 19 bytes with its first chunk** (`EncodedAudioChunkMetadata.decoderConfig`). It is not sent, and the decoder took the track without one. Whether it is to travel is an open question; this is a fact for it, not a decision.
- The demo, headless: the publishing page sends the picture and a tone on two tracks; the watching page decoded 200 pieces of audio (the last with a sample of 0.20, which is what the tone has) beside 120 frames, and its `AudioContext` ran after the click on "sound".

## Limitations and blockers

- **Nobody heard it.** That the watching page sounds what it decodes was not checked by ear, and a headless browser has no output.
- No microphone was run.
- Nothing keeps audio and video in step, and nothing buffers: a piece is played after the one before it.
- No loss, no reordering, no slow subscriber was run; the publisher leaves a chunk out when 25 wait (provisional).
- Not through the moqtail relay. Not on Linux.

## Open questions raised

- Whether the Opus `description` is sent (LOC-04 has an Audio Config property for it) and given to the decoder.
- A Group and a stream for every 20 ms is 50 streams a second for one track: what that costs a relay next to video was not looked at.
