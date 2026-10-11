# The demo in Playwright's Firefox and WebKit

- Date: 2026-10-11
- Commit SHA: the commit that adds this record; nothing was changed for the runs
- Roadmap item: none closed. Facts for which browsers can publish and watch

## Objective

Open the publishing and the watching page of the demo in the Firefox and the WebKit that Playwright installs, with Chromium as the other end, and note what works. Nothing was mended.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browsers | Playwright's builds (`playwright-core` 1.64.0), headless: Firefox 157.0 (build 1555), WebKit 27.2 (build 2251, "mac14 special"), Chromium 156.0.8078.4. **None is a shipped browser** |
| The demo | `bun demo/serve.ts`: the own relay with a self-signed certificate given to the pages by its hash, the rooms of the spike, H.264 640x360 and Opus |

## Commands

```sh
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384
# a script outside the repository: in each browser, what it says it has; /watch against Chromium's /publish; /publish with Chromium's /watch
```

## Observed results

| | Firefox 157.0 (Playwright) | WebKit 27.2 (Playwright) |
|---|---|---|
| `WebTransport` | There | **Not run**: `browser.newPage()` failed, "Protocol error (Page.overrideSetting): Unknown setting: PushAPIEnabled" |
| The relay's self-signed certificate by its hash (`serverCertificateHashes`) | Taken: the page connected | Not run |
| `VideoEncoder`, H.264 in Annex B form (`avc1.42E01F`) | Says supported, and encoded: 71 frames in 3 seconds | Not run |
| `VideoDecoder`, `avc3.42E01F` without a description | Says supported, and decoded Chromium's frames | Not run |
| `AudioEncoder` and `AudioDecoder`, Opus | Both say supported, and both ran | Not run |
| `MediaStreamTrackProcessor` (the camera and the microphone of the demo) | **Not there** | Not run |
| Watches what Chromium publishes | **Yes**: 170 frames and 280 pieces of audio in 6 seconds; the first frame after 109 ms; a frame there 3 ms after it was made (median), 50 at most | Not run |
| Publishes, and Chromium watches | **Yes**: 127 frames and 225 pieces in 5 seconds, nothing dropped | Not run |
| The room: the WebSocket, the chat, the count | The page found the broadcast through the room both ways | Not run |

- **Firefox's audio carried times about 100 ms ahead**: Chromium, watching Firefox, found a piece of audio there 101 ms before the time it carries (median; 8 ms before with Chromium as the publisher). The page gives the encoder the same times in both browsers; what Firefox's Opus encoder does with them was not looked into.
- The Joining FETCH of the first viewer was refused in every run, as it is for a first viewer (the relay has nothing yet).

## Limitations and blockers

- **WebKit could not be run at all**, as on 2026-10-10 ([record](2026-10-10-browser-support-webtransport-webcodecs.md), where `newPage()` did not return): Playwright's WebKit build for macOS 14 and `playwright-core` 1.64.0 do not work together here. After the failure the script did not end and was stopped. Safari itself (17.4 on this machine) is older than the first Safari with WebTransport.
- Playwright's Firefox is not the shipped Firefox.
- One run, headless; nothing was seen or heard. The camera and the microphone were not tried in any browser.
