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

## 2026-10-12: the parts that came since, in Firefox

One run with Playwright's Firefox 157.0 as the viewer and Chromium 156.0.8078.4 as the publisher, and one the other way round, for what went into the packages on 2026-10-12: `createPlayout`, `keepPlaying` and `keepPublishing`, the MSF catalog, and the two trials `?ladder=1` and `?abr=1`. Nothing was mended. WebKit was not tried again.

```sh
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384
node spikes/lossy-link/proxy.mjs --listen 45386 --to 45382 --control 45387
# a script outside the repository: /publish?picture=busy&bitrate=2500000&ladder=1&via=45386 and /watch?via=45386&buffer=150&abr=1;
# 5 s as it is, 8 s with rate=1000 and delay=50, 12 s without, the link cut for 10 s (loss=1), 9 s back
```

| | Firefox watches (Chromium publishes) | Firefox publishes (Chromium watches) |
|---|---|---|
| The catalog: two video tracks in one alternate group and the audio track, with `samplerate` and `channelConfig` | Read; the tracks were found by it | Written, and read by Chromium as Chromium writes it |
| `createPlayout`: picture and sound by one clock | 30 frames a second shown, 136 ms after they were made with a buffer of 150 ms; 50 pieces of sound a second to the loudspeaker, 1 break | Chromium showed 29 frames a second and sounded 49 to 50 pieces a second; **the picture was shown 14 ms after it was made, with a buffer of 150 ms** (below) |
| `?ladder=1`: two encoders | | Both ran, 28 to 29 frames a second, 51 key frames each; 1,364 frames of the large version and 1,358 of the small |
| `?abr=1`: the version changes | Down to `video-low` (first frame 1.1 s after it was decided) and up again (76 ms) | Chromium went down (1.8 s) and up again (47 ms) |
| `keepPlaying` | "playing (session 2)" within 9 s of the link being back, 30 frames and 50 pieces a second | |
| `keepPublishing` | | "publishing (session 2)", and Chromium played again |
| The decoder | Did not fail | Did not fail (Chromium's) |

- **Everything ran in Firefox, on either end.**
- **With Firefox as the publisher the buffer does nothing for the picture**: Chromium showed a frame 14 ms after it was made where the buffer was 150 ms, the frame having waited 4 ms. `createPlayout` counts picture and sound from the fastest of either, and Firefox's Opus carries times about 100 ms ahead (above, from 2026-10-11): the sound sets the clock, and the picture is late by that much before it begins. Picture and sound are then that far apart as well. Not looked into further.
- **The numbers while the link was narrow say nothing about a browser**: in this script the publisher went through the same proxy as the viewer, whose `rate` is shared by all, so 1 Mbit/s was less than the publisher needed to send. That the version changed and came back is what the row has.
- One run each, headless; nothing was seen or heard.
