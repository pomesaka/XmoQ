# WebCodecs loopback: encode, copy to bytes, decode in one page

- Date: 2026-10-10
- Commit SHA: the commit that adds this record. The runs were made on its working copy while it was on top of `f7d9ab2ac22a`; the spike directory did not change afterwards
- Roadmap item: groundwork for Phase 1 ("Browser WebCodecs publisher", "Decode and render video", "Test late join, keyframes"). No roadmap item is closed by this record.

## Objective

Collect facts for the open "Codec/profile, Track/Object packaging, audio pipeline" decision, using WebCodecs alone: no network and no MoQ. Nothing is decided here. Every codec string, resolution, bitrate, frame rate and frame count below is a provisional value chosen to make the check run.

Questions:

1. Which codecs `VideoEncoder` and `VideoDecoder` report as supported, by codec string, resolution and `hardwareAcceleration`.
2. Whether frames that are encoded, copied to bytes and decoded again come back with the same count, order and timestamps.
3. How key frames are requested, when `decoderConfig` arrives, and what a decoder does when it starts at a delta or at a later key.
4. Encoded size per frame and time per frame.
5. What has to travel with an `EncodedVideoChunk`.

Opus was run in the same form after the video runs.

## Environment

| Item | Value |
|---|---|
| Machine | Apple M3 Pro, 11 logical cores, 18 GB, macOS 14.4 (23E214), arm64 |
| Browser A | Chrome for Testing 156.0.8078.4 (Playwright build 1248), headless (`--headless`, the full browser binary) |
| Browser B | chrome-headless-shell 156.0.8078.4 (Playwright build 1248) |
| Browser C | Google Chrome 155.0.8059.39, headless |
| Driver | `playwright-core` 1.64.0, Bun 1.3.12, `typescript` 6.0.3, `@types/bun` 1.4.2 |
| Page | `http://127.0.0.1:<port>`, cross-origin isolated (`crossOriginIsolated` was `true`), experiments on the window's main thread |
| MoQ, certificates | not used |

Hardware acceleration as observed, not assumed:

| Browser | WebGL renderer | `chrome://gpu` | Platform (VideoToolbox) codecs used |
|---|---|---|---|
| A | ANGLE Metal, Apple M3 Pro | Video Decode: Hardware accelerated; Video Encode: Hardware accelerated | yes |
| B | SwiftShader (software Vulkan) | page could not be opened (`net::ERR_FAILED`) | none: every `prefer-hardware` query returned unsupported |
| C | ANGLE Metal, Apple M3 Pro | Video Decode: Hardware accelerated; Video Encode: Hardware accelerated | yes |

The encoder and decoder implementation of each run was read from the DevTools `Media` domain (`kVideoEncoderName`, `kIsPlatformVideoEncoder`, and the decoder equivalents). WebCodecs itself does not expose it.

Specifications read on 2026-10-10: WebCodecs, W3C Working Draft of 7 October 2026 (cited as WC); the AVC, HEVC and AV1 registrations of 8 June 2026; VP8 of 9 September 2024; VP9 of 14 May 2025; Opus of 24 September 2026. The registrations state that they are non-normative.

## Commands

From `spikes/webcodecs-loopback/`. Each was run as written, with absolute paths.

```sh
bun install --frozen-lockfile
bun run typecheck

CHROMIUM_PATH="$HOME/Library/Caches/ms-playwright/chromium-1248/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing" \
  bun run.ts --label chrome-for-testing-156-headless                       # browser A
CHROMIUM_PATH="$HOME/Library/Caches/ms-playwright/chromium_headless_shell-1248/chrome-headless-shell-mac-arm64/chrome-headless-shell" \
  bun run.ts --label headless-shell-156                                    # browser B
CHROMIUM_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  bun run.ts --label chrome-155-headless                                   # browser C

bun report.ts out/<label>.json
```

The three runs were made one after another, not in parallel. `run.ts` starts and stops the page server and the browser. The raw results are in `out/<label>.json`, which is not committed; the tables below were produced by `report.ts` from those files. What one run does is described in the [spike README](../../spikes/webcodecs-loopback/README.md).

Provisional settings: 30 frames per second, bitrate of 0.1 bit per pixel per frame (2 764 800 bit/s at 1280x720), 90 frames per loopback run, `keyFrame: true` on frames 0, 30 and 60, `bitrateMode` left at its default. The picture is generated on a canvas and is not camera content.

## Observed results

### 1. Reported support

Browser A. Each cell is `no-preference` / `prefer-hardware` / `prefer-software`.

`VideoEncoder.isConfigSupported`:

| Codec string | Meaning | 640x360 | 1280x720 | 1920x1080 | 3840x2160 |
|---|---|---|---|---|---|
| `avc1.42001f` | Baseline, level 3.1 | yes / yes / yes | yes / yes / yes | no / no / no | no / no / no |
| `avc1.42E01F` | Constrained Baseline, level 3.1 | yes / yes / yes | yes / yes / yes | no / no / no | no / no / no |
| `avc1.4D401F` | Main, level 3.1 | yes / yes / yes | yes / yes / yes | no / no / no | no / no / no |
| `avc1.64001F` | High, level 3.1 | yes / yes / yes | yes / yes / yes | no / no / no | no / no / no |
| `avc1.640033` | High, level 5.1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `avc3.42E01F` | Constrained Baseline, level 3.1, avc3 | yes / yes / yes | yes / yes / yes | no / no / no | no / no / no |
| `hvc1.1.6.L93.B0` | Main, level 3.1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `hev1.1.6.L93.B0` | Main, level 3.1, hev1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `hvc1.1.6.L153.B0` | Main, level 5.1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `hvc1.2.4.L93.B0` | Main 10, level 3.1 | no / no / no | no / no / no | no / no / no | no / no / no |
| `vp8` |  | yes / no / yes | yes / no / yes | yes / no / yes | yes / no / yes |
| `vp09.00.31.08` | Profile 0, level 3.1, 8 bit | yes / no / yes | yes / no / yes | yes / no / yes | yes / no / yes |
| `vp09.00.51.08` | Profile 0, level 5.1, 8 bit | yes / no / yes | yes / no / yes | yes / no / yes | yes / no / yes |
| `vp09.02.31.10` | Profile 2, level 3.1, 10 bit | yes / no / yes | yes / no / yes | yes / no / yes | yes / no / yes |
| `av01.0.05M.08` | Main, level 3.1, 8 bit | yes / no / yes | yes / no / yes | yes / no / yes | yes / no / yes |
| `av01.0.13M.08` | Main, level 5.1, 8 bit | yes / no / yes | yes / no / yes | yes / no / yes | yes / no / yes |
| `av01.0.05M.10` | Main, level 3.1, 10 bit | no / no / no | no / no / no | no / no / no | no / no / no |

`VideoDecoder.isConfigSupported`:

| Codec string | Meaning | 640x360 | 1280x720 | 1920x1080 | 3840x2160 |
|---|---|---|---|---|---|
| `avc1.42001f` | Baseline, level 3.1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `avc1.42E01F` | Constrained Baseline, level 3.1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `avc1.4D401F` | Main, level 3.1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `avc1.64001F` | High, level 3.1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `avc1.640033` | High, level 5.1 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `avc3.42E01F` | Constrained Baseline, level 3.1, avc3 | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `hvc1.1.6.L93.B0` | Main, level 3.1 | yes / yes / no | yes / yes / no | yes / yes / no | yes / yes / no |
| `hev1.1.6.L93.B0` | Main, level 3.1, hev1 | yes / yes / no | yes / yes / no | yes / yes / no | yes / yes / no |
| `hvc1.1.6.L153.B0` | Main, level 5.1 | yes / yes / no | yes / yes / no | yes / yes / no | yes / yes / no |
| `hvc1.2.4.L93.B0` | Main 10, level 3.1 | yes / yes / no | yes / yes / no | yes / yes / no | yes / yes / no |
| `vp8` |  | yes / no / yes | yes / no / yes | yes / no / yes | yes / no / yes |
| `vp09.00.31.08` | Profile 0, level 3.1, 8 bit | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `vp09.00.51.08` | Profile 0, level 5.1, 8 bit | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `vp09.02.31.10` | Profile 2, level 3.1, 10 bit | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `av01.0.05M.08` | Main, level 3.1, 8 bit | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `av01.0.13M.08` | Main, level 5.1, 8 bit | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |
| `av01.0.05M.10` | Main, level 3.1, 10 bit | yes / yes / yes | yes / yes / yes | yes / yes / yes | yes / yes / yes |

No query rejected. Differences in the other browsers:

- **Browser B** (no GPU): every `prefer-hardware` cell is `no`. All HEVC strings are `no` for both encoder and decoder, under every preference. H.264, VP8, VP9 and AV1 are otherwise the same as in A.
- **Browser C**: all 408 answers are the same as in A.

What the tables show:

- **H.264 encoder support depends on the level in the codec string.** The level 3.1 strings are refused at 1920x1080 and above; `avc1.640033` (level 5.1) is accepted at all four sizes. The decoder accepted every H.264 string at every size. The HEVC, VP9 and AV1 level did not change the encoder's answer.
- **Hardware encoding on this machine exists for H.264 and HEVC only.** VP8, VP9 and AV1 encode with `prefer-hardware` is unsupported; VP9 and AV1 decode with `prefer-hardware` is supported, VP8 decode is not.
- **HEVC has no Chromium software implementation here.** With `prefer-software` the HEVC decoder is unsupported, and in browser B, where the platform codecs are unavailable, HEVC is absent altogether. With `prefer-software` the HEVC encoder is reported as supported and runs as `VideoToolbox` with `kIsPlatformVideoEncoder=false`; which implementation that is inside VideoToolbox was not confirmed.
- **10 bit**: the encoder refused HEVC Main 10 and AV1 10 bit, and accepted VP9 Profile 2. Not run further.
- `prefer-hardware` returned unsupported whenever no hardware codec was used. WC §7.9 calls the two preferences hints that a user agent "may ignore … for any reason", so this is Chromium's behaviour, not a guarantee of the API.

### 2. Loopback

Each output `EncodedVideoChunk` was copied with `copyTo` into a `Uint8Array`, a new `EncodedVideoChunk` was built from `type`, `timestamp`, `duration` and those bytes, and a `VideoDecoder` was configured from a `VideoDecoderConfig` rebuilt from the emitted `decoderConfig` (its `description` copied to bytes as well).

A run "matches" when the number of decoded frames equals the number of input frames, output timestamps are strictly increasing, every timestamp and duration equals the input frame's, and the frame number read back from the decoded picture equals the input frame's.

| Browser | Loopback runs | Result |
|---|---|---|
| A | 30: H.264 (OpenH264 and VideoToolbox; `avc` and `annexb`), HEVC (both preferences; `hevc` and `annexb`), VP8, VP9, AV1; 640x360, 1280x720, 1920x1080; `latencyMode` `realtime` and `quality` | all 30 match, both when chunks were submitted one at a time and all at once |
| B | 16: H.264 (OpenH264), VP8, VP9, AV1 | all 16 match |
| C | 30: the same set as A | all 30 match |

- Every run produced exactly one chunk per input frame, in input order, with the input frame's `timestamp` and `duration`. No frame was dropped, including with `latencyMode: "realtime"`, for which WC §7.11 allows drops. Frames were submitted as fast as the encoder took them, not at 30 per second, so this does not show what happens under real-time load.
- Timestamps were `round(i × 1 000 000 / 30)` µs (33 333, 66 667, 100 000, …) and came back unchanged. WC §4.6 ("Output VideoFrames") copies `timestamp` and `duration` from the chunk.
- Implementations seen: `OpenH264VideoEncoder`, `VpxVideoEncoder` (VP8 and VP9), `Av1VideoEncoder`, `VideoToolbox`; `FFmpegVideoDecoder` (H.264), `VpxVideoDecoder`, `Dav1dVideoDecoder`, `VideoToolboxVideoDecoder`.
- The coded size of a decoded `VideoFrame` is not always the encoded size, and the pixel format depends on the decoder. Display size equalled the encoded size in every run.

- FFmpegVideoDecoder avc1.42E01F 1280x720: I420 coded 1280x738
- FFmpegVideoDecoder avc1.42E01F 640x360: I420 coded 640x386
- VideoToolboxVideoDecoder avc1.42E01F 1280x720: NV12 coded 1280x720
- VideoToolboxVideoDecoder avc1.42E01F 640x360: NV12 coded 640x360
- VideoToolboxVideoDecoder hvc1.1.6.L93.B0 1280x720: NV12 coded 1280x720
- VideoToolboxVideoDecoder hvc1.1.6.L93.B0 640x360: NV12 coded 640x360
- VideoToolboxVideoDecoder hvc1.1.6.L93.B0 1920x1080: NV12 coded 1920x1080
- VpxVideoDecoder vp8 1280x720: I420 coded 1280x720
- VpxVideoDecoder vp8 640x360: I420 coded 640x360
- VpxVideoDecoder vp8 1920x1080: I420 coded 1920x1080
- VpxVideoDecoder vp09.00.31.08 1280x720: I420 coded 1344x720
- VpxVideoDecoder vp09.00.31.08 640x360: I420 coded 704x360
- VpxVideoDecoder vp09.00.31.08 1920x1080: I420 coded 1984x1080
- Dav1dVideoDecoder av01.0.05M.08 1280x720: I420 coded 1280x720
- Dav1dVideoDecoder av01.0.05M.08 640x360: I420 coded 640x360
- Dav1dVideoDecoder av01.0.05M.08 1920x1080: I420 coded 1920x1080

### 3. Key frames and decoderConfig

#### Requesting key frames

`VideoEncoderConfig` has no member for a key frame interval (WC §7.8). The only control is `encode(frame, { keyFrame: true })` per frame; WC §7.13 says `true` means the frame "MUST be encoded as a key frame" and `false` leaves the choice to the user agent.

| Case | Browser A | Browser B | Browser C |
|---|---|---|---|
| `keyFrame: true` on frames 0, 30, 60 of 90 | key chunks exactly at 0, 30, 60, every encoder | same | same |
| `keyFrame` never requested, 300 frames, both latency modes | key chunk at 0 only, every encoder | VP8, VP9, AV1: 0 only. **OpenH264: 0, 40, 64, 192** | key chunk at 0 only, every encoder |

So an encoder can emit a key frame that was not requested, and in ten seconds of input most encoders emitted none on their own. `EncodedVideoChunk.type` agreed with the frame type parsed from the bytes in every run (H.264, HEVC, VP8, VP9; not parsed for AV1).

#### When decoderConfig arrives

In every run without reconfiguration, `metadata.decoderConfig` was present on **chunk 0 only**. It was not repeated on later key chunks. This is what WC §6.6 ("Output EncodedVideoChunks") specifies: the config is attached only when it differs from the previous one. `decoderConfig` was the only metadata member seen (`svc` and `alphaSideData` need settings that were not used).

Members emitted by Chromium: `codec` (the configured string), `codedWidth`, `codedHeight`, `colorSpace` (`bt709` primaries and matrix, `iec61966-2-1` transfer, `fullRange: false` in every run), `hardwareAcceleration: "no-preference"`, `rotation: 0`, `flip: false`, and `description` as below.

| Encoder, format | `description` | Key chunk contains | Delta chunk contains |
|---|---|---|---|
| H.264 `avc` (default) | 29–54 bytes, `AVCDecoderConfigurationRecord` | IDR slices only (NAL type 5); no SPS or PPS | non-IDR slices (type 1) |
| H.264 `annexb` | absent | SPS (7), PPS (8), IDR slices, on **every** key chunk | non-IDR slices |
| HEVC `hevc` (default) | 105–115 bytes, `HEVCDecoderConfigurationRecord` | IDR only (type 19 or 20) | type 1 |
| HEVC `annexb` | absent | VPS (32), SPS (33), PPS (34), IDR, on every key chunk | type 1 |
| VP8, VP9 | absent | (not parsed beyond the frame type) | |
| AV1 | absent | temporal delimiter, sequence header, frame (OBU types 2, 1, 6) on every key chunk | temporal delimiter, frame (2, 6) |

The `description` bytes differ between encoders and between settings for the same codec string: OpenH264 and VideoToolbox, `realtime` and `quality` on VideoToolbox, and each resolution all gave different bytes. OpenH264 at 1280x720 emitted four slices per frame. VideoToolbox with `latencyMode: "quality"` put an SEI unit (type 6 for H.264, 39 for HEVC) in front of the first key frame.

#### Reconfiguring the encoder mid-stream

60 frames, `keyFrame` never requested, `VideoEncoder.configure()` called again before frame 30.

| Change | Encoders | Chunk 30 | New `decoderConfig` on chunk 30 |
|---|---|---|---|
| half the bitrate | all | delta (no key frame) | no |
| half the width and height | OpenH264, VideoToolbox (H.264, HEVC), VP8 | **key**, not requested | yes |
| half the width and height | VP9, AV1 | **delta** | yes |

Decoding the size-change streams:

| Stream | Decoder configured once | Decoder reconfigured when the new `decoderConfig` arrives |
|---|---|---|
| H.264 `avc`, HEVC `hevc` | 30 frames, then `EncodingError` through the error callback, decoder `closed` | 60/60 match |
| H.264 `annexb`, HEVC `annexb`, VP8 | 60/60 match | 60/60 match |
| VP9, AV1 | 60/60 match | 30 frames; `decode()` throws `DataError` for chunks 30–59, because `configure()` makes a key chunk required and chunk 30 is a delta |

A new `decoderConfig` is therefore not a signal that the chunk it comes with is a key chunk. Browser B gave the same outcomes for the codecs it has, except that its OpenH264 runs also had the unrequested key chunk at 40 noted above. Browser C gave the same outcomes as A.

#### Decoder scenarios

All at 1280x720, `realtime`, key chunks at 0, 30, 60. The outcome was the same for every codec, format, decoder and browser unless a row says otherwise. Decoded pictures were also compared, as 64x36 thumbnails, with the pictures of the plain full decode.

| Scenario | Outcome |
|---|---|
| New decoder, first chunk is a delta (chunk 1) | `decode()` throws `DataError` synchronously ("A key frame is required after configure() or flush().") for each of chunks 1–29. The decoder stays `configured`. From key chunk 30 on, 60/60 frames match, pictures identical. |
| New decoder, starts at key chunk 30, config taken from chunk 0's metadata | 60/60 match, pictures identical. |
| New decoder, starts at delta chunk 31 relabelled `type: "key"` | `decode()` throws `DataError` ("… was marked as type `key` but wasn't a key frame."; for Annex B streams the message suggests filling in `description` instead). Rejected until key chunk 60; then 30/30 match. Chromium checked the label against the bytes for all five codecs. |
| `flush()` after 10 chunks, then delta chunk 10 | `DataError` for chunks 10–29, as after `configure()`. Then 60 frames from chunk 30. |
| One delta (chunk 5) left out, **H.264** (FFmpeg and VideoToolbox) and **VP8** | No error, no exception, 89 frames out with the timestamps they were given. Pictures differ from the full decode from output 5 to output 28, that is until the next key chunk; the frame number read from the picture was wrong on at most 5 of them. |
| One delta left out, **HEVC** (VideoToolbox) and **VP9** | `EncodingError` ("Decoding error.") through the error callback at the first chunk after the gap; decoder `closed`. 5 frames out. |
| One delta left out, **AV1** (dav1d) | 5 frames out, then `EncodingError` and `closed`. The error surfaced after later chunks had been submitted; when exactly was not measured. |
| Chunks built without `duration` | 90/90 decode, pictures identical, every `VideoFrame.duration` is `null`. |
| Config reduced to `codec` and `description` (no size, no `colorSpace`) | 90/90 match. Pictures identical in browser A. In browser B the VP8 and VP9 pictures differed slightly on every frame (mean difference 6.7 of 255 per channel); H.264 and AV1 were identical. |
| `avc` or `hevc` stream, config without `description` | Every `decode()` throws `DataError`; nothing is decoded. The decoder stays `configured`. |
| `optimizeForLatency: true` | 90/90 match, pictures identical. Its effect on timing is in the next section. |

These follow WC §4.5: `configure()` and `flush()` set `[[key chunk required]]`, `decode()` then throws `DataError` for a chunk whose `type` is not `key`, and "Implementers SHOULD inspect the chunk's [[internal data]] to verify that it is truly a key chunk". A decode failure after that goes to the error callback and closes the decoder (§4.6). What the specification does not say, and the runs show, is whether a missing delta is detected at all: for H.264 and VP8 it was not.

Browser C: the same implementations were used as in A, and every scenario had the same outcome as in A.

### 4. Size and time

Conditions for every number below: the machine and browsers above, headless, one run each, generated picture (gradient, moving bar, large digits, marker cells), 90 frames with key frames at 0, 30, 60, chunks submitted from the window's main thread, `performance.now()` on a cross-origin isolated page. The figures are not targets and no pass mark was set. The generated picture is much easier to compress than camera video, so the sizes say little about real content.

Sizes are bytes and times are milliseconds, each as min / median / p95 / max. The number of chunks is in parentheses.

- **Serial**: one frame is submitted and its output awaited before the next; the time is from `encode()` or `decode()` to the output callback, so it includes the hop to the codec thread and back.
- **Held**: the decoder did not output any of the first three chunks within 1000 ms while nothing further was submitted. The run then stopped waiting, and there is no per-frame time.
- **Burst**: all frames are submitted, then `flush()`; total time divided by frames. For encoding this includes drawing the canvas and creating the `VideoFrame`.
- `(true)` and `(false)` after a name are `kIsPlatformVideoEncoder` and `kIsPlatformVideoDecoder`.

Browser A:

| Run | Encoder / decoder | latencyMode | Bitrate set | Key bytes (n) | Delta bytes (n) | Measured bit/s | Encode serial ms | Encode burst ms/frame | Decode serial ms | Decode serial ms, optimizeForLatency | Decode burst ms/frame |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `avc1.42E01F` avc 1280x720 | OpenH264 (false) / FFmpeg (false) | realtime | 2764800 | 5623 / 9490 / 9752 / 9752 (3) | 5629 / 11034 / 14916 / 16633 (87) | 2654469 | 5.84 / 6.74 / 7.34 / 25.80 | 4.50 | held | 0.42 / 0.55 / 0.72 / 0.85 | 0.45 |
| `avc1.42E01F` annexb 1280x720 | OpenH264 (false) / FFmpeg (false) | realtime | 2764800 | 5653 / 9520 / 9782 / 9782 (3) | 5629 / 10937 / 14941 / 16762 (87) | 2646376 | 5.80 / 6.79 / 7.86 / 31.58 | 5.29 | held | 0.40 / 0.53 / 0.64 / 0.95 | 0.45 |
| `avc1.42E01F` avc 1280x720 | OpenH264 (false) / FFmpeg (false) | quality | 2764800 | 5623 / 9493 / 9702 / 9702 (3) | 5629 / 10926 / 15021 / 16695 (87) | 2648485 | 5.31 / 5.78 / 7.93 / 39.52 | 5.25 | held | not run | 0.44 |
| `avc1.42E01F` avc 640x360 | OpenH264 (false) / FFmpeg (false) | realtime | 691200 | 2273 / 4059 / 4240 / 4240 (3) | 2267 / 2889 / 3197 / 3411 (87) | 691739 | 4.96 / 5.31 / 7.26 / 12.83 | 3.01 | held | not run | 0.19 |
| `avc1.42E01F` avc 1280x720 | VideoToolbox (true) / VideoToolbox (true) | realtime | 2764800 | 5822 / 7464 / 7583 / 7583 (3) | 5380 / 10661 / 15682 / 18298 (87) | 2525365 | 7.63 / 8.47 / 10.21 / 270.59 | 4.50 | 0.60 / 0.74 / 1.05 / 66.83 | 0.64 / 0.92 / 1.14 / 22.02 | 0.66 |
| `avc1.42E01F` annexb 1280x720 | VideoToolbox (true) / VideoToolbox (true) | realtime | 2764800 | 5853 / 7495 / 7614 / 7614 (3) | 5380 / 10661 / 15682 / 18298 (87) | 2525613 | 7.61 / 8.24 / 8.69 / 36.17 | 6.09 | 0.78 / 0.99 / 1.50 / 21.06 | 0.74 / 0.91 / 1.14 / 22.72 | 0.68 |
| `avc1.42E01F` avc 1280x720 | VideoToolbox (true) / VideoToolbox (true) | quality | 2764800 | 7017 / 10505 / 12191 / 12191 (3) | 5773 / 11605 / 18755 / 19948 (87) | 2839845 | 6.00 / 6.88 / 8.06 / 41.72 | 3.03 | held | not run | 0.67 |
| `avc1.42E01F` avc 640x360 | VideoToolbox (true) / VideoToolbox (true) | realtime | 691200 | 2060 / 2707 / 2816 / 2816 (3) | 1676 / 2702 / 3606 / 4465 (87) | 656805 | 5.20 / 5.91 / 7.18 / 39.32 | 2.88 | 0.56 / 0.67 / 0.97 / 21.15 | not run | 0.63 |
| `hvc1.1.6.L93.B0` hevc 1280x720 | VideoToolbox (false) / VideoToolbox (true) | realtime | 2764800 | 18922 / 24261 / 24897 / 24897 (3) | 7911 / 11374 / 14077 / 15383 (87) | 2863277 | 38.84 / 112.36 / 129.15 / 404.00 | 116.55 | 0.71 / 0.86 / 1.09 / 13.67 | 0.72 / 0.88 / 1.05 / 7.42 | 0.26 |
| `hvc1.1.6.L93.B0` annexb 1280x720 | VideoToolbox (false) / VideoToolbox (true) | realtime | 2764800 | 19001 / 24340 / 24976 / 24976 (3) | 7911 / 11374 / 14077 / 15383 (87) | 2863909 | 37.34 / 113.96 / 131.60 / 444.51 | 109.58 | 0.70 / 0.85 / 1.08 / 6.36 | 0.71 / 0.86 / 1.02 / 7.03 | 0.25 |
| `hvc1.1.6.L93.B0` hevc 1280x720 | VideoToolbox (false) / VideoToolbox (true) | quality | 2764800 | 18922 / 24261 / 24897 / 24897 (3) | 7911 / 11374 / 14077 / 15383 (87) | 2863277 | 37.82 / 114.84 / 168.94 / 415.60 | 108.31 | 0.73 / 0.85 / 1.06 / 5.69 | not run | 0.25 |
| `hvc1.1.6.L93.B0` hevc 640x360 | VideoToolbox (false) / VideoToolbox (true) | realtime | 691200 | 3500 / 4711 / 6198 / 6198 (3) | 1957 / 2887 / 3536 / 3832 (87) | 708163 | 37.32 / 55.71 / 70.04 / 209.99 | 54.23 | 0.49 / 0.54 / 1.41 / 5.99 | not run | 0.23 |
| `hvc1.1.6.L93.B0` hevc 1920x1080 | VideoToolbox (false) / VideoToolbox (true) | realtime | 6220800 | 40172 / 50489 / 54062 / 54062 (3) | 17751 / 26248 / 32098 / 34704 (87) | 6439888 | 70.03 / 190.52 / 226.03 / 453.43 | 183.48 | 0.88 / 1.23 / 1.58 / 6.95 | not run | 0.44 |
| `hvc1.1.6.L93.B0` hevc 1280x720 | VideoToolbox (true) / VideoToolbox (true) | realtime | 2764800 | 2533 / 3764 / 3973 / 3973 (3) | 1311 / 8497 / 10266 / 10507 (87) | 1887509 | 7.58 / 9.22 / 11.22 / 58.58 | 7.19 | 0.63 / 0.80 / 1.40 / 8.47 | 0.65 / 0.77 / 0.96 / 6.92 | 0.28 |
| `hvc1.1.6.L93.B0` annexb 1280x720 | VideoToolbox (true) / VideoToolbox (true) | realtime | 2764800 | 2621 / 3852 / 4061 / 4061 (3) | 1311 / 8497 / 10266 / 10507 (87) | 1888213 | 8.39 / 9.00 / 10.55 / 34.80 | 6.66 | 0.67 / 0.78 / 1.06 / 5.61 | 0.64 / 0.76 / 0.92 / 6.78 | 0.25 |
| `hvc1.1.6.L93.B0` hevc 1280x720 | VideoToolbox (true) / VideoToolbox (true) | quality | 2764800 | 3521 / 7085 / 7459 / 7459 (3) | 4306 / 7021 / 9706 / 10275 (87) | 1720933 | 5.79 / 7.40 / 8.95 / 50.15 | 3.58 | 0.74 / 1.01 / 1.40 / 8.46 | not run | 0.25 |
| `hvc1.1.6.L93.B0` hevc 640x360 | VideoToolbox (true) / VideoToolbox (true) | realtime | 691200 | 993 / 1481 / 1589 / 1589 (3) | 623 / 2558 / 3738 / 4333 (87) | 603416 | 5.82 / 6.58 / 8.39 / 45.50 | 3.02 | 0.49 / 0.55 / 0.85 / 7.40 | not run | 0.28 |
| `hvc1.1.6.L93.B0` hevc 1920x1080 | VideoToolbox (true) / VideoToolbox (true) | realtime | 6220800 | 4397 / 6511 / 6993 / 6993 (3) | 2509 / 15985 / 19452 / 20071 (87) | 3563347 | 14.24 / 16.20 / 17.78 / 58.75 | 13.60 | 0.92 / 1.12 / 1.58 / 11.40 | not run | 0.44 |
| `vp8` 1280x720 | Vpx (false) / Vpx (false) | realtime | 2764800 | 3683 / 5349 / 6023 / 6023 (3) | 2005 / 12296 / 19776 / 24068 (87) | 2619192 | 3.82 / 4.82 / 5.49 / 16.85 | 2.73 | 0.59 / 1.72 / 2.36 / 2.62 | 0.55 / 1.70 / 2.32 / 2.66 | 1.59 |
| `vp8` 1280x720 | Vpx (false) / Vpx (false) | quality | 2764800 | 3683 / 5349 / 6023 / 6023 (3) | 2005 / 12296 / 19776 / 24068 (87) | 2619192 | 3.73 / 5.01 / 13.41 / 23.92 | 2.95 | 0.57 / 1.69 / 2.30 / 2.71 | not run | 1.62 |
| `vp8` 640x360 | Vpx (false) / Vpx (false) | realtime | 691200 | 1423 / 2235 / 2347 / 2347 (3) | 946 / 3039 / 4344 / 4834 (87) | 698883 | 3.46 / 4.03 / 4.51 / 9.52 | 2.68 | 0.25 / 0.70 / 0.87 / 0.94 | not run | 0.51 |
| `vp8` 1920x1080 | Vpx (false) / Vpx (false) | realtime | 6220800 | 7073 / 9588 / 11190 / 11190 (3) | 3706 / 25733 / 35320 / 42757 (87) | 5249101 | 4.77 / 6.52 / 7.62 / 24.29 | 5.03 | 1.14 / 3.42 / 4.17 / 4.59 | not run | 3.04 |
| `vp09.00.31.08` 1280x720 | Vpx (false) / Vpx (false) | realtime | 2764800 | 3761 / 11461 / 14851 / 14851 (3) | 2451 / 6747 / 15017 / 23799 (87) | 1752744 | 4.07 / 4.87 / 6.09 / 16.37 | 3.35 | 0.32 / 0.44 / 0.78 / 1.80 | 0.30 / 0.45 / 0.75 / 0.84 | 0.44 |
| `vp09.00.31.08` 1280x720 | Vpx (false) / Vpx (false) | quality | 2764800 | 3761 / 11461 / 14851 / 14851 (3) | 2451 / 6747 / 15017 / 23799 (87) | 1752744 | 4.25 / 5.02 / 6.59 / 27.89 | 3.10 | 0.31 / 0.44 / 0.75 / 0.82 | not run | 0.44 |
| `vp09.00.31.08` 640x360 | Vpx (false) / Vpx (false) | realtime | 691200 | 1738 / 4980 / 6695 / 6695 (3) | 821 / 1733 / 3497 / 5538 (87) | 459995 | 3.80 / 4.68 / 6.24 / 16.17 | 3.04 | 0.16 / 0.23 / 0.36 / 0.81 | not run | 0.22 |
| `vp09.00.31.08` 1920x1080 | Vpx (false) / Vpx (false) | realtime | 6220800 | 6214 / 19451 / 24012 / 24012 (3) | 5222 / 14481 / 41467 / 65346 (87) | 3976789 | 5.63 / 6.78 / 8.98 / 55.34 | 6.09 | 0.63 / 0.88 / 1.48 / 1.91 | not run | 0.83 |
| `av01.0.05M.08` 1280x720 | Av1 (false) / Dav1d (false) | realtime | 2764800 | 2463 / 7062 / 9534 / 9534 (3) | 2003 / 6910 / 9474 / 11981 (87) | 1581440 | 4.22 / 5.20 / 5.97 / 13.35 | 3.51 | held | 0.46 / 0.77 / 1.06 / 1.33 | 0.48 |
| `av01.0.05M.08` 1280x720 | Av1 (false) / Dav1d (false) | quality | 2764800 | 2446 / 7137 / 9623 / 9623 (3) | 1666 / 6182 / 8775 / 10112 (87) | 1415411 | 5.00 / 5.97 / 7.18 / 18.17 | 4.36 | held | not run | 0.53 |
| `av01.0.05M.08` 640x360 | Av1 (false) / Dav1d (false) | realtime | 691200 | 1140 / 3250 / 4212 / 4212 (3) | 855 / 2315 / 3060 / 5393 (87) | 550936 | 4.30 / 5.07 / 6.36 / 9.19 | 2.78 | held | not run | 0.34 |
| `av01.0.05M.08` 1920x1080 | Av1 (false) / Dav1d (false) | realtime | 6220800 | 3787 / 11722 / 16509 / 16509 (3) | 2633 / 12824 / 17880 / 20757 (87) | 2855685 | 6.79 / 8.05 / 9.47 / 16.54 | 7.22 | held | not run | 1.03 |

Browser B (software codecs only, no GPU):

| Run | Encoder / decoder | latencyMode | Bitrate set | Key bytes (n) | Delta bytes (n) | Measured bit/s | Encode serial ms | Encode burst ms/frame | Decode serial ms | Decode serial ms, optimizeForLatency | Decode burst ms/frame |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `avc1.42E01F` avc 1280x720 | OpenH264 (false) / FFmpeg (false) | realtime | 2764800 | 6112 / 10039 / 10724 / 10724 (3) | 5938 / 10915 / 14515 / 15733 (87) | 2639048 | 2.88 / 3.45 / 4.72 / 18.39 | 6.38 | held | 0.38 / 0.48 / 0.60 / 0.78 | 0.48 |
| `avc1.42E01F` annexb 1280x720 | OpenH264 (false) / FFmpeg (false) | realtime | 2764800 | 6142 / 10069 / 10728 / 10728 (3) | 5938 / 10942 / 14611 / 15554 (87) | 2641784 | 2.83 / 3.58 / 5.57 / 25.92 | 5.78 | held | 0.44 / 0.57 / 0.90 / 1.07 | 0.47 |
| `avc1.42E01F` avc 1280x720 | OpenH264 (false) / FFmpeg (false) | quality | 2764800 | 6112 / 10039 / 10724 / 10724 (3) | 5938 / 10989 / 14506 / 15581 (87) | 2636312 | 3.01 / 3.41 / 4.29 / 21.03 | 5.81 | held | not run | 0.44 |
| `avc1.42E01F` avc 640x360 | OpenH264 (false) / FFmpeg (false) | realtime | 691200 | 2464 / 4389 / 4722 / 4722 (3) | 2305 / 2856 / 3181 / 3301 (87) | 691896 | 2.42 / 2.74 / 4.34 / 8.49 | 3.30 | held | not run | 0.13 |
| `vp8` 1280x720 | Vpx (false) / Vpx (false) | realtime | 2764800 | 4107 / 5791 / 6633 / 6633 (3) | 2106 / 12452 / 18962 / 21039 (87) | 2630645 | 0.97 / 1.68 / 2.18 / 9.05 | 4.07 | 0.55 / 1.61 / 2.25 / 2.39 | 0.53 / 1.62 / 2.20 / 2.46 | 1.53 |
| `vp8` 1280x720 | Vpx (false) / Vpx (false) | quality | 2764800 | 4107 / 5791 / 6633 / 6633 (3) | 2106 / 12452 / 18962 / 21039 (87) | 2630645 | 1.04 / 1.74 / 2.29 / 3.81 | 3.95 | 0.54 / 1.61 / 2.18 / 2.51 | not run | 1.60 |
| `vp8` 640x360 | Vpx (false) / Vpx (false) | realtime | 691200 | 1559 / 2390 / 2636 / 2636 (3) | 1083 / 3036 / 4381 / 4901 (87) | 702904 | 0.58 / 0.90 / 1.85 / 5.04 | 1.38 | 0.21 / 0.46 / 0.64 / 0.69 | not run | 0.46 |
| `vp8` 1920x1080 | Vpx (false) / Vpx (false) | realtime | 6220800 | 7419 / 9417 / 12288 / 12288 (3) | 3569 / 26569 / 38511 / 45784 (87) | 5712397 | 2.51 / 4.01 / 5.28 / 7.74 | 9.06 | 0.96 / 3.48 / 4.30 / 4.97 | not run | 3.14 |
| `vp09.00.31.08` 1280x720 | Vpx (false) / Vpx (false) | realtime | 2764800 | 4372 / 11886 / 15874 / 15874 (3) | 2439 / 6669 / 13003 / 25422 (87) | 1724339 | 1.69 / 2.35 / 3.40 / 13.17 | 4.54 | 0.30 / 0.43 / 0.65 / 1.29 | 0.29 / 0.42 / 0.65 / 0.93 | 0.44 |
| `vp09.00.31.08` 1280x720 | Vpx (false) / Vpx (false) | quality | 2764800 | 4372 / 11886 / 15874 / 15874 (3) | 2439 / 6669 / 13003 / 25422 (87) | 1724339 | 1.69 / 2.38 / 3.56 / 9.43 | 4.68 | 0.32 / 0.43 / 0.67 / 0.94 | not run | 0.43 |
| `vp09.00.31.08` 640x360 | Vpx (false) / Vpx (false) | realtime | 691200 | 1876 / 5114 / 7370 / 7370 (3) | 779 / 1685 / 3332 / 5676 (87) | 462611 | 1.33 / 2.05 / 3.18 / 3.77 | 2.62 | 0.16 / 0.22 / 0.33 / 0.69 | not run | 0.22 |
| `vp09.00.31.08` 1920x1080 | Vpx (false) / Vpx (false) | realtime | 6220800 | 6557 / 19213 / 26355 / 26355 (3) | 5250 / 14071 / 43070 / 73546 (87) | 4069061 | 3.26 / 4.43 / 6.08 / 22.34 | 9.68 | 0.61 / 0.86 / 1.77 / 2.08 | not run | 0.88 |
| `av01.0.05M.08` 1280x720 | Av1 (false) / Dav1d (false) | realtime | 2764800 | 2670 / 7873 / 10613 / 10613 (3) | 2383 / 7048 / 10232 / 12303 (87) | 1657123 | 1.73 / 2.38 / 3.37 / 13.22 | 4.50 | held | 0.43 / 0.69 / 1.30 / 2.11 | 0.55 |
| `av01.0.05M.08` 1280x720 | Av1 (false) / Dav1d (false) | quality | 2764800 | 2676 / 7936 / 10756 / 10756 (3) | 2048 / 6388 / 9412 / 10586 (87) | 1479781 | 2.52 / 3.36 / 4.29 / 9.27 | 5.63 | held | not run | 0.62 |
| `av01.0.05M.08` 640x360 | Av1 (false) / Dav1d (false) | realtime | 691200 | 1173 / 3297 / 4438 / 4438 (3) | 925 / 2275 / 2938 / 5455 (87) | 557536 | 1.70 / 2.16 / 2.95 / 3.61 | 2.82 | held | not run | 0.35 |
| `av01.0.05M.08` 1920x1080 | Av1 (false) / Dav1d (false) | realtime | 6220800 | 4002 / 12621 / 18130 / 18130 (3) | 2775 / 12684 / 17416 / 20767 (87) | 2833515 | 4.25 / 5.52 / 7.01 / 10.30 | 10.89 | held | not run | 1.15 |

What the tables show:

- **FFmpeg (H.264) and dav1d (AV1) hold frames back by default.** Without `optimizeForLatency: true` they did not emit a frame until more input or a flush arrived; with it, each frame came out in under a millisecond (median). The VideoToolbox decoder did the same on the H.264 stream that VideoToolbox encoded with `latencyMode: "quality"`, and emitted frames one by one on the `realtime` stream. WC §1 allows a codec to "emit new outputs only when new inputs are provided". How many frames are held was not measured.
- **HEVC with `prefer-software` is an order of magnitude slower** than every other encoder here (median 112 ms per frame at 1280x720 against 5–9 ms for the others, and 190 ms at 1920x1080) and its key chunks are about six times the size of the `prefer-hardware` ones.
- Key chunks were not always larger than delta chunks with this picture and bitrate (H.264 and VP8 key chunks were smaller than the median delta). In browser A the measured bitrate ranged from 57% to 104% of the configured one.
- The first frame of a hardware encoder took longer than the rest (the maximum column), up to 270 ms for VideoToolbox H.264.

Browser C: same implementations as A; its times are in `out/chrome-155-headless.json` and are not reproduced here.

### 5. What travels with an EncodedVideoChunk

This lists what a receiver needed in these runs. Where each item goes in MoQ is not addressed.

Per chunk (WC §8.2):

| Item | Needed | Observed |
|---|---|---|
| Chunk bytes | yes | `copyTo` output, fed back unchanged |
| `type` (`key` or `delta`) | yes, required by `EncodedVideoChunkInit` | The decoder refuses a delta after `configure()` or `flush()`, and Chromium refuses a delta labelled as key |
| `timestamp`, microseconds, signed 64 bit | yes, required | Returned unchanged on the decoded frame; the decoder does not derive it from the bitstream |
| `duration`, microseconds | optional | Returned unchanged; when omitted, `VideoFrame.duration` is `null` and decoding is unaffected |

Per stream, from `decoderConfig`:

| Item | Needed | Observed |
|---|---|---|
| `codec` string | yes, the only required member (WC §7.6) | Emitted equal to the encoder's string |
| `description` | yes for H.264 `avc` and HEVC `hevc`; does not exist for `annexb`, VP8, VP9, AV1 | Without it nothing decodes. Emitted once, on chunk 0, and again only after a size change |
| `codedWidth`, `codedHeight` | no | Decoding matched without them |
| `colorSpace` | no for decoding | Without it, VP8 and VP9 pictures differed slightly in browser B |
| `hardwareAcceleration`, `rotation`, `flip` | not examined | Emitted as `no-preference`, `0`, `false`. `hardwareAcceleration` is the receiver's own preference in `VideoDecoderConfig` |

Not part of a chunk or of `decoderConfig`, but needed by a receiver in these runs:

- **Where a key chunk is.** A new decoder can only start at one, and so can a decoder after `flush()`.
- **The current `decoderConfig`, for a receiver that joins after chunk 0.** WebCodecs emits it once. With `avc` and `hevc` formats a late joiner cannot decode without the `description`; with `annexb`, VP8, VP9 and AV1 the key chunk itself was enough, given the codec string.
- **Whether a chunk is missing.** The H.264 and VP8 decoders gave no error for a gap and produced damaged pictures until the next key chunk. HEVC, VP9 and AV1 decoders closed. Either way the decoder does not report which chunk was missing.
- **Whether the config changed**, and for VP9 and AV1 that a changed config does not come with a key chunk.

`optimizeForLatency` and `hardwareAcceleration` are choices of the receiver and do not travel.

### 6. Opus

`AudioEncoder` and `AudioDecoder`, codec string `opus`, 48 000 Hz, a generated 440 Hz sine, `f32` interleaved. Both `isConfigSupported` calls returned supported. The three browsers gave identical chunk sizes, timestamps and decoder outputs.

| Run | Input | Output chunks | Chunk bytes (min / median / p95 / max) |
|---|---|---|---|
| 2 channels, 96 000 bit/s | 100 × 960 frames (2.00 s) | 101, each 20 000 µs, timestamps 0, 20 000, … 2 000 000 | 241 / 241 / 241 / 376 |
| 2 channels, 96 000 bit/s | 100 × 1024 frames (2.13 s) | 107, each 20 000 µs | 241 / 241 / 241 / 376 |
| 1 channel, 32 000 bit/s | 100 × 960 frames | 101, each 20 000 µs | 70 / 81 / 84 / 154 |

- Every chunk has `type: "key"`, as the Opus registration §4 states. A new decoder started at chunk 10 without error.
- The encoder regroups input into 20 ms chunks whatever the size of the `AudioData` it is given, and emits one chunk more than the input duration covers.
- `decoderConfig` came on chunk 0 only: `codec`, `sampleRate`, `numberOfChannels`, and a 19-byte `description` that starts with `OpusHead` and carries a pre-skip of 312 samples. The encoder was left at its default `opus` format.
- The `description` changes what the decoder outputs. With it, the first `AudioData` has 648 frames instead of 960 and later timestamps are 6500 µs earlier (0, 13 500, 33 500, …); the total is 96 648 frames for 101 chunks. Without it, every `AudioData` has 960 frames and timestamps are 0, 20 000, 40 000, …; the total is 96 960. This matches the registration's §5: the pre-skip in the description sets the number of priming samples to discard, and without a description it is 0.
- A decoder that started at chunk 10 with the `description` also trimmed 312 samples from its first output, although that chunk is not the start of the stream.
- Encode and decode times were not measured for audio.

## Limitations and blockers

No blocker. Limits of what was run:

- One machine (Apple M3 Pro, macOS 14.4) and Chromium-based browsers only, all headless. A browser with a window, Firefox, Safari, another OS and other hardware were not tried. Hardware support differs by machine, so the support tables do not transfer.
- The picture is generated. Sizes, the key-to-delta ratio and encode times will differ for camera or screen content.
- Frames were not paced at the frame rate, so frame dropping in `realtime` mode, `encodeQueueSize` and backpressure were not exercised.
- One run per configuration; no repeat statistics. Times include the message hop between the page and the codec.
- 90 frames per run, 300 for the unrequested-key check. Whether an encoder inserts key frames over a longer time was not checked.
- Everything ran on the window's main thread; a Worker was not tried.
- Not run: `scalabilityMode` (temporal layers), `bitrateMode` other than the default, `alpha`, 10 bit, `contentHint`, resolutions above 1920x1080 for the loopback, HEVC and AV1 `quantizer` options, hardware VP9 or AV1 encoding (not available here).
- The number of frames a decoder holds back, and the moment the AV1 decoder reports a gap, were not measured.
- AV1 chunk labels were not checked against the bitstream; only OBU types were listed.
- Only one loss pattern was tried: a single delta chunk, the sixth of a group.
- Audio: Opus only, no timing, no packet loss, `opus` format only.
- The `description` bytes were not parsed beyond the length-size field.

## Open questions raised

For the user to decide. They belong to the existing "Codec/profile, Track/Object packaging, audio pipeline" and "Supported browsers" entries in [open questions](../open-questions.md), which this record does not edit.

1. **Codec.** On this machine H.264 is the only codec with a Chromium software encoder, a hardware encoder and decoders in both forms. VP8, VP9 and AV1 encode in software only. HEVC exists only through the platform and disappears without it. The H.264 level in the codec string limits the resolution the encoder accepts.
2. **H.264 and HEVC bitstream format.** With `avc` or `hevc`, parameter sets live only in a `description` that is emitted once, so a late joiner and a size change both depend on delivering it. With `annexb` every key chunk carried them and a decoder configured once kept working across a size change.
3. **How a late joiner gets the decoder configuration**, since the encoder emits it on the first chunk only.
4. **Key frame interval.** It is the publisher's choice, frame by frame. A receiver that joins or loses a chunk waits for the next key chunk. Whether a receiver can ask for one is a transport question.
5. **What a receiver does on a gap.** H.264 and VP8 decoders show damaged pictures without any error; HEVC, VP9 and AV1 decoders close and have to be created again.
6. **`latencyMode` on the publisher and `optimizeForLatency` on the receiver.** Without the latter, the H.264 and AV1 software decoders held frames back.
7. **Whether `duration` is carried.** Decoding does not need it.
8. **Whether resolution changes mid-stream are allowed**, given that VP9 and AV1 change size without a key frame while the other encoders insert one.
9. **Opus `description`.** Sending it makes the decoder trim 312 samples and shift timestamps by 6.5 ms, also for a decoder that starts mid-stream.
10. **Which browsers and machines to check next.** Everything above is Chromium on one Mac.
