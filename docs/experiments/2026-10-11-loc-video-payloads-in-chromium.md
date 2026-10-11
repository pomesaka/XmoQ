# LOC video payloads in Chromium: `avc3`, and frames of moqlivemock's publisher

- Date: 2026-10-11
- Commit SHA: the commit that adds `@xmoq/loc` together with this record. The runs were made on its working copy, on `main` at `da703d45` (the merge of the change that reads a catalog over a connection), with the change that publishes a catalog below it
- Roadmap item: groundwork for Phase 1 ("Browser WebCodecs publisher", "Decode and render video"). It closes no item

## Objective

Before anything encodes or plays video, three things about the payload of a LOC video Object were not known:

1. whether the encoder and the decoder of Chromium take a codec string that begins `avc3`, which LOC-04 §2.1.1 names among the formats that "also support parameter sets in the bitstream payload", and which is what `moqlivemock`'s catalog has. Every earlier run here used `avc1.42E01F`;
2. what form the video payloads of `moqlivemock`'s publisher have, in bytes that are kept (an earlier survey had read the first eight);
3. whether Chromium decodes those payloads, as they come and after `toAnnexB` of `@xmoq/loc`.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64 |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless: the build the interop run uses |
| Publisher | `mlmpub` of `Eyevinn/moqlivemock` v0.16.1 (commit `91d903e9`), unchanged |
| Drafts | draft-ietf-moq-loc-04; draft-ietf-moq-transport-18 |

## Commands

Throwaway pages, driven with Playwright from outside the repository; nothing of them is kept. The capture of the frames is in `fixtures/loc-04/README.md`.

```sh
mise run check
```

## Observed results

### 1. The codec strings

For each of `avc1.42E01F`, `avc3.42E01F`, `avc1.4D401F`, `avc3.4D401F`, `avc1.640028`, `avc3.640028`, at 1280x720, 1 Mbit/s, 30 frames a second, `latencyMode: "realtime"`, with `avc: { format: "annexb" }` and with `avc: { format: "avc" }`: `VideoEncoder.isConfigSupported`, five frames of a canvas encoded with a key frame first, `VideoDecoder.isConfigSupported` for the same string, and the five chunks decoded. One run.

| | All twelve |
|---|---|
| `VideoEncoder.isConfigSupported` | supported |
| Encoded | 5 chunks, the first a key chunk |
| `decoderConfig.codec` the encoder reported | the string it was configured with, `avc3` where that was `avc3` |
| `VideoDecoder.isConfigSupported`, and the decode | supported; 5 frames |

- With `annexb`: the key chunk begins `00 00 00 01 27` (a start code of 4 bytes and a sequence parameter set), and `decoderConfig.description` is absent.
- With `avc`: the key chunk begins with a length (`00 00 0a 9f 25` for `42E01F`), and `description` is present.
- **`avc1` and `avc3` made no difference to any of it.** Which of the two a catalog names is not decided by what Chromium takes.
- The headless build has an H.264 encoder and decoder.

### 2. The payloads of `mlmpub`

27 Objects of `video_400kbps_avc`. Every Object has one property, type `0x10` with the wall-clock time in microseconds, 40 ms apart. **Every payload has a length of 4 bytes before each NAL unit and no start code**: the key frame is three NAL units (25, 4 and 21,492 bytes, first bytes `67`, `68`, `65`), the others one. `mlmpub` sends a Group on one stream and makes the Group ID of the time in seconds. `fixtures/loc-04/` has two of the payloads and the bytes of the stream they came on.

### 3. Decoding them

A decoder configured with `{ codec: "avc3.4D401F", optimizeForLatency: true }` and no description, given the key frame and the frame after it.

| The payloads | Seen |
|---|---|
| As `mlmpub` sent them | `decode()` throws `DataError`: "A key frame is required after configure() or flush(). If you're using AVC formatted H.264 you must fill out the description field in the VideoDecoderConfig." No frame |
| After `toAnnexB` | Two frames, 1280x720 |

So a viewer that plays LOC video of this publisher has to turn the lengths into start codes, or give the decoder a description, which `mlmpub` does not send.

### The checks

`mise run check` passed on the working copy the runs were made on: 42 tests of `@xmoq/loc`, 1103 of `@xmoq/moqt`, 207 of `@xmoq/msf`, none failing; the Rust, layer and workflow checks passed.

## Limitations and blockers

- One run each, one machine, the headless build; no hardware encoder was asked for or looked at.
- The pictures that were decoded were not looked at, only that two frames of the right size came out.
- Two frames of one track of one publisher. A whole Group, and a second Group, were not decoded.
- What a NAL unit type means was not read from the H.264 specification.

## Open questions raised

- Which of `avc1` and `avc3` an own catalog names for H.264 in Annex B form: Chromium takes both, LOC-04 §2.1.1 names `avc3` for parameter sets in the payload, and `moqlivemock` writes `avc3`.
