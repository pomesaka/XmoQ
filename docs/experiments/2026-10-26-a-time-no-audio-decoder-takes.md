# A time no audio decoder takes

- Date: 2026-10-26
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, audio: `playAudioTrack` of `@xmoq/media` handed the time of every Object to `EncodedAudioChunk` as it was, where the video's receiver dropped one over 2^53 - 1 microseconds.

## Objective

An audio track whose first Object has a LOC Timestamp beyond what a decoder of WebCodecs takes (`timestamp` is a `required [EnforceRange] long long`, WebCodecs §8.1): does the viewer drop that one and play what comes after it?

**Loopback, one machine.**

## Expected

Written before the first run, in the head of `interop/video.ts` (case `audio-beyond-time`): the viewer drops the first Object as `beyond-time`, decodes the 50 after it, received 51, and its decoder does not fail. Before `MAX_DECODER_TIME` was looked at, the first Object threw as its chunk was made, and nothing after it was decoded.

## Environment

macOS 14.4, arm64, Chromium of Playwright 1.64.0, headless; the own relay (debug build) on 31500. `bun interop/matrix.ts --relay own --base-port 31500 --only "audio:"` and `--only "an Object whose time no decoder takes"`. A page serves the track Object by Object (`serveTrack`, `sendObjectIn`), the payload `f8 ff fe` (one Opus frame of 20 ms), the first with the Timestamp 2^60 and the 50 after it 20 ms apart from the driver's clock.

## Result

**As expected.** The case passed in its first run (1.2 s) and again on the next `main` (1.1 s): `beyond-time` 1, 50 pieces decoded, no decoder error.

With `src/web/audio-track.ts` as it is on `main` before this change, it failed as it should: received 1, 0 pieces, nothing dropped. The reading of the subscription stopped at the first Object.

The case `audio` (a tone encoded and played page to page) failed once in five runs of this change: one piece was dropped as `late`, and piece 94 began 20 ms after where the one before it ended. The other four passed. Whether it fails so without this change was not looked at; a piece dropped as `late` is decided before the chunk is made, which this change does not touch.

## Not looked at

- A time beyond the limit in the middle of a track: by the code it is dropped as `beyond-time` as well, since the limit is looked at before `createFastestWay`; only the first Object was sent so. Before this change only the first could reach the decoder: a later one was `not-of-the-clock`.
- `decoder-refused`: no chunk the decoder throws for was sent.
- A person hearing it.
