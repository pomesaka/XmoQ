# The demo at 720 and 1080 lines

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: none closed. The publishing page's size can be chosen; which sizes are offered, and at which bitrates, is not decided.

## Objective

The demo sent 640x360 and nothing else. With `/publish?size=360|720|1080`: do the larger sizes run, does a key frame reach the 1,000,000 bytes an Object may have, does the encoder keep up, does a viewer show 30 frames a second, and what does it cost?

Expected, written before the runs: all three run in Chromium; the clock's key frames are far below the limit at every size, and the busy picture's nearer to it at 1080 lines but below; the encoder keeps up; the CPU grows with the size.

## What was built

`SIZES` in `demo/shared.ts`: 640x360, 1280x720 and 1920x1080, 30 frames a second. The picture is painted in the units of the smallest and scaled; a camera is asked for the size. The catalog has the size's width, height, codec and bitrate, and the watching page plays by the catalog as before.

- **The level** is the lowest of H.264 that has the size at 30 frames a second, by Table A-1 as `libavcodec/h264_levels.c` of FFmpeg has it (read on 2026-10-12; the Recommendation itself was not read): Level 3.1 allows 3,600 macroblocks a frame and 108,000 a second, which 1280x720 at 30 is exactly; 1920x1080 is 8,160 and 244,800, which Level 4 allows (8,192 and 245,760). So `avc1.42E01F` for 360 and 720, and `avc1.42E028` for 1080.
- **The bitrate** without `?bitrate=`: 1.5, 3 and 6 Mbit/s. **Provisional**: the 360's doubled and doubled again, from no measurement of what looks well.

## Environment

macOS 14.4, arm64, loopback. Chromium 156.0.8078.4 (playwright-core 1.64.0), headless, with its fake camera (`--use-fake-device-for-media-stream`). The own relay as `bun demo/serve.ts` starts it. One browser has the publishing page and one watching page; the CPU is that of all its processes over 10 s, by `ps`.

## Result

One run each, 15 s of publishing. The limit of an Object is 1,000,000 bytes.

| Size | Picture | Bitrate asked for | Encoded, kbit/s | Largest chunk (a key frame), bytes | Frames a second in, and shown by the viewer | CPU, of one core |
|---|---|---|---|---|---|---|
| 640x360 | clock | 1.5 Mbit/s | 421 | 5,855 | 30, 30 | 22% |
| 640x360 | busy | 1.5 Mbit/s | 1,515 | 19,555 | 30, 30 | 28% |
| 640x360 | fake camera | 1.5 Mbit/s | 326 | 4,287 | 20, 20 | 18% |
| 1280x720 | clock | 3 Mbit/s | 882 | 13,566 | 30, 30 | 38% |
| 1280x720 | busy | 3 Mbit/s | 2,976 | 36,954 | 30, 30 | 56% |
| 1280x720 | fake camera | 3 Mbit/s | 914 | 11,543 | 20, 20 | 26% |
| 1920x1080 | clock | 6 Mbit/s | 1,326 | 21,261 | 30, 30 | 55% |
| 1920x1080 | busy | 6 Mbit/s | 5,956 | 47,406 | 30, 30 | 85% |
| 1920x1080 | fake camera | 6 Mbit/s | 1,244 | 16,723 | 20, 20 | 55% |
| 1920x1080 | busy | 20 Mbit/s | 13,901 | 66,157 | 30, 30 | 84% |

- **All three sizes ran**, with the catalog's codec (`avc3.42E01F`, and `avc3.42E028` at 1080 lines) and size as sent. The camera gave the size it was asked for.
- **No key frame came near the limit**: the largest was 66,157 bytes, a fifteenth of it, at 1080 lines with the busy picture at the most Level 4 allows (20 Mbit/s asked for, 13.9 encoded). `onKeyChunksNotSent` was not called in any run. A key frame over the limit was seen before only with half the picture random pixels at 30 Mbit/s ([record](2026-10-11-pages-send-and-play-video.md)). So the limit was not touched, and nothing about it was changed.
- **The encoder kept up**: no frame was left out as `encoder-behind` in any run.
- **The viewer showed what was sent**, 30 frames a second, 52 to 60 ms after they were made with the buffer of 50 ms, nothing dropped, the decoder never failing; the sound as before.
- **The fake camera gives 20 frames a second at every size**, 640x360 as well; that is the device, not the size.
- **The CPU grows with the size and with how busy the picture is**: from 22% of a core to 55% for the clock, and to 85% for the busy picture at 1080 lines, for publisher and viewer together.
- `bun demo/check.ts` at the default size says what it said before (30 frames and 50 pieces of sound a second, 0 breaks in 4 s), and passes with `&size=1080` too.

## Not done

- A real camera, whose picture has noise an encoder cannot make small: its key frames will be larger than the fake camera's bars. Not measured.
- A link: 6 Mbit/s over anything but loopback, with `?ladder=1` (whose small version stays 320x180), and the viewer in a terminal.
- Firefox and a phone at these sizes; a machine that is slower than this one.
- How the three look. Nobody looked.
