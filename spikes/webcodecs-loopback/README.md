# WebCodecs loopback spike

Encodes a generated picture with WebCodecs in a real Chromium, copies every `EncodedVideoChunk` to a byte array, and decodes those bytes again in the same page. No network and no MoQ. It collects facts for the open codec and packaging decisions; it does not make them, and every codec string, resolution, bitrate and frame count in `run.ts` is a provisional value for this check.

The run record is [docs/experiments/2026-10-10-webcodecs-loopback.md](../../docs/experiments/2026-10-10-webcodecs-loopback.md).

## Files

| File | Role |
|---|---|
| `page.ts` | Runs in the browser: support probe, encode, decode scenarios, Opus |
| `run.ts` | Bundles `page.ts`, serves it on `127.0.0.1`, drives it with Playwright, writes `out/<label>.json` |
| `report.ts` | Turns one `out/<label>.json` into Markdown |
| `types.ts` | Result shapes shared by the three |

## Run

Bun 1.3.12 was used. `run.ts` does not download a browser; `CHROMIUM_PATH` points at an existing Chromium or Chrome binary.

```sh
cd spikes/webcodecs-loopback
bun install --frozen-lockfile
bun run typecheck

CHROMIUM_PATH="<path to a Chromium or Chrome binary>" bun run.ts --label <label>
bun report.ts out/<label>.json
```

Options of `run.ts`:

| Option | Meaning |
|---|---|
| `--label <name>` | Required. Name of the output file |
| `--headed` | Launch with a window instead of headless |
| `--chrome-arg=<switch>` | Extra Chromium switch, repeatable |
| `--frames <n>` | Frames per loopback run (default 90) |
| `--timeout <seconds>` | Limit for one experiment (default 300) |
| `--skip-audio` | Leave out the Opus runs |

One run takes roughly ten minutes on the machine in the record, because decoders that hold frames back are waited on for one second per frame before the wait is given up. The browser and the page server are stopped when `run.ts` ends.

## What one run does

1. Reads `chrome://gpu` and the page's environment (user agent, WebGL renderer, `crossOriginIsolated`).
2. Calls `VideoEncoder.isConfigSupported` and `VideoDecoder.isConfigSupported` for every codec string, resolution and `hardwareAcceleration` value in `run.ts`.
3. For each codec family and each of `prefer-software` and `prefer-hardware` that the encoder reports as supported:
   - a loopback run with all decode scenarios (start at a delta, start at a later key, drop one delta, flush, no `duration`, reduced config, no `description`);
   - loopback runs at two more resolutions and with `latencyMode: "quality"`;
   - a 300-frame encode in which `keyFrame` is never requested;
   - two runs in which the encoder is reconfigured halfway (bitrate, size).
4. Encodes and decodes a generated sine with Opus.

The picture is drawn on an `OffscreenCanvas`: a moving gradient, the frame number as text, and the same number as a row of black and white cells. After decoding, the cells are read back, so each decoded picture can be matched to the frame that went in. The name of the encoder and decoder implementation comes from the DevTools `Media` domain.
