# A muted page sends silence

- Date: 2026-10-27
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, meetings: a button of the meeting page that stops sending the person's voice.

## Objective

The meeting page mutes its microphone with `track.enabled = false`. Does the track then go on giving pieces of sound, and silent ones, so that the audio track goes on having Objects? Does a page in the meeting go on receiving them? And what does Chromium do with noise suppression, echo cancellation and automatic gain control where a page does not ask for them?

**Loopback, one machine.**

## Expected

Said before the runs (in the session, not in a file): a disabled audio track gives silence at the same pace, and the three are on where nothing is asked.

## Environment

macOS 14.4, Chromium of Playwright 1.64.0 (`playwright-core` 1.64.0), headless, with `--use-fake-device-for-media-stream` (a fake microphone that beeps) and `--use-fake-ui-for-media-stream`. A demo of its own: `bun demo/serve.ts --relay-port 35443 --port 35080 --rooms-port 35787`.

## Commands

A page at `http://localhost`: `getUserMedia({ audio: { sampleRate: 48000, channelCount: 1 } })`, and once more with the three asked for as `true`; the track's `getSettings()`; its `MediaStreamTrackProcessor` read for 3 s, `enabled = false` from 1 s to 2 s, with the pieces of each second counted and the loudest sample of each.

Two pages of `/meet` of the demo, p1 and p2, in a room of their own: the pieces of sound p2 received (`others` of `data-meet`) were counted for 3 s while p1 sent, 3 s after p1 pressed `mute`, and 3 s after it pressed it again.

## Observed results

**As expected.** The track gave 100 pieces in the second before, 100 while disabled and 101 after; the loudest sample was 1 before, 0 while disabled and 0.99997 after. `getSettings()` had the three `true` with them asked for and without.

p2 received 150 pieces of p1's sound in each 3 s: sending, muted and sending again. The button said `mute` (`aria-pressed` false) while p1 sent, and `unmute` (true) while it did not.

## Limitations and blockers

- That what p2 received while p1 was muted was silent was not measured on p2: the silence was seen where the track gives it.
- A real microphone, and a browser other than Chromium of Playwright, were not tried; what Chromium does with the three without a page asking may differ there.
- The transcriber was not run with a muted page.

## Open questions raised

None.
