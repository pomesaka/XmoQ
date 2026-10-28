# A camera off and a muted microphone send nothing

- Date: 2026-10-28
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, meetings: the bar of controls of the demo's pages, where a camera turned off and a muted microphone send nothing.

## Objective

A page whose camera is turned off ends its video Group with an End of Group and sends nothing until the camera is on again (`pause` of `createVideoTrackEncoder`), and a muted page sends no sound, with the time of the pause put into the times after it (`pause` of `createAudioTrackEncoder`). Does a viewer then show no picture and sound nothing, and the picture and the sound again after, without the sound coming too late? Does the session to publish stay while nothing is sent?

This replaces the mute that sent silence ([record](2026-10-27-a-muted-page-sends-silence.md)): the person chose that what is not said is not sent.

**Loopback, one machine.**

## Expected

Said before the runs (to the PM, not in a file): no picture within a second of the camera off (`DEFAULT_HIDE_AFTER_MS`), none of the sound while muted, both again within a second after, and the session kept by the relay's keep-alive (the relay sends a PING after 3 s at the latest without a packet, `crates/moqt-io/src/wt.rs`; transport §13.6.1).

## Environment

macOS 14.4, Chromium of Playwright 1.64.0 (`playwright-core` 1.64.0), headless, with `--use-fake-device-for-media-stream` (a fake camera of 20 frames a second and a fake microphone) and `--use-fake-ui-for-media-stream`. A demo of its own: `bun demo/serve.ts --relay-port 35443 --port 35080 --rooms-port 35787`.

## Commands

`/publish?source=camera` (the tone) and `/watch` in a room of their own: the counts of the watching page's report and its middle pixel over 3 s with all on, 4 s with the camera off, 3 s on again, 4 s muted, and twice 3 s after.

`/meet?source=camera` (p1) and `/meet` (p2) in a room of their own: what p2 received of p1 (`others` of `data-meet`) and the middle pixel of p2's tile of p1, over 3 s each with all on, the camera off, on again, muted, unmuted; then 40 s with the camera off and muted; then 4 s with both on.

The breaks of the sound of `/watch` with the fake microphone were counted with the code of `main` before these changes and with them: 3 s twice with the tone, then twice with `?audio=mic`.

## Observed results

**As expected.** `/publish` and `/watch`:

| While | Video Objects sent | Shown a second | Too late | Sounded a second | Breaks | Middle pixel |
|---|---|---|---|---|---|---|
| all on, 3 s | 121 in all | 20 | 0 | 50 | +0 | the picture |
| camera off, 4 s | +2 | 0 | 0 | 50 | +0 | 0,0,0 |
| camera on again, 3 s | +65 | 20 | 0 | 50 | +0 | the picture |
| muted, 4 s | +80 | 20 | 0 | 0 | +0 | the picture |
| unmuted, first 3 s | +61 | 20 | 5 | 50 | +1 | the picture |
| unmuted, 3 s after | +59 | 20 | 0 | 50 | +0 | the picture |

The two Objects after the camera went off are the frame under way and the End of Group. Five frames came too late, and the sound had one break, in the 3 s after the sound came back; none after.

`/meet`:

| While | Frames of p1 that p2 got | Pieces of sound | p2's tile of p1 | p1 |
|---|---|---|---|---|
| all on, 3 s | 45 | 150 | the picture | publishing |
| camera off, 3 s | 7 | 150 | 0,0,0 | publishing |
| camera on again, 3 s | 44 | 150 | the picture | publishing |
| muted, 3 s | 45 | 3 | the picture | publishing |
| unmuted, 3 s | 45 | 146 | the picture | publishing |
| camera off and muted, 40 s | 2 | 8 | 0,0,0 | publishing |
| both on again, 4 s | 56 | 187 | the picture | publishing |

The breaks of the fake microphone were there before: with the code of `main`, the tone had +0 and +0 in 3 s, and the fake microphone +325 and +337, its loudest sample 0.000; with these changes, +0 and +0, and +364 and +356. What makes them was not looked into.

## Limitations and blockers

- That p1's session to publish was not closed and opened again during the 40 s was not seen: the page says `publishing` again after a new session too.
- A real camera and microphone, and the terminal viewer with a page whose camera goes off, were not run here. The terminal viewer's picture removed with the kitty protocol (`d=I`) was checked by the bytes it writes, not in a terminal.
- The five frames too late after the sound came back were not looked into.

## Open questions raised

None.
