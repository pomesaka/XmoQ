# The demo with the publishing page in a background tab: a frame a second, and no sound

- Date: 2026-10-11
- Commit SHA: the commit that changes the pages of the demo, together with this record
- Roadmap item: none. Two things a person saw when the demo was opened: "the viewer's frame rate is low", and "sound does nothing"

## Objective

Find why a person saw few frames a second and heard nothing, mend it in the demo, and make both show in numbers a script can check.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | A person: Chrome, the two pages as tabs of one window, then as two windows. Here: Chromium 156.0.8078.4 (playwright-core 1.64.0), headless and with a window |
| Relay, rooms | own; the rooms of the spike |

## Commands

```sh
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384
bun demo/check.ts http://localhost:45381 "&timer=slow"   # the publisher with the timers of a background tab
bun demo/check.ts http://localhost:45381
mise run check
```

## Observed results

- **The cause, as the person found**: with the two pages in two windows the picture was smooth. The publishing page had been a tab in the background, where a browser calls a page's timers once a second at most.
- **A background tab could not be made here**: with Playwright, headless or with a window, both tabs said `visibilityState` "visible" whichever was in front, and the publisher made 30 frames a second either way. So the page got `?timer=slow`, its own timer at once a second, to see what that does:

| The publishing page | Frames a second, made and shown | Pieces of audio a second to the loudspeaker | Too late | What leaves for the loudspeaker |
|---|---|---|---|---|
| With the timers of a background tab (`?timer=slow`) | 1 and 1 | **0** | 50 | **silence**, 0.00 of full scale |
| As it is now: the clock in a worker | 30 and 30 | 50 | 0 | 0.25 of full scale between 220 and 660 Hz, the beep at 1,008 Hz |

- **Both things the person saw are this one cause.** The sound is made by the clock, as many pieces as have passed: once a second that is 50 pieces at once, and all but the last few are past their time when they come, so the watching page sounds none.
- **Mended in the demo**: what the pages do on a timer (paint and encode, make the tone, write the numbers) is now started by a worker that posts a message every so many milliseconds. A worker's timers are not slowed with the page, and a message is handled at once. **Not seen in a real background tab**, for the reason above: a person has to look.
- **The watching page keeps 30 decoded frames at most** while it waits to draw: in a background tab nothing is drawn, and before this every frame was kept with its picture.
- **What leaves for the loudspeaker is now measured**: everything that sounds goes through one node that is listened to. The page says how many pieces a second go out, how many come too late, how loud it is and at which pitch, and "SILENT" where it is. `demo/check.ts` fails for fewer than 25 frames a second shown, fewer than 40 pieces a second sounded, a level under 0.1, or a pitch outside 150 to 1,200 Hz; with `&timer=slow` it fails on all four.
- The tone is louder than it was: 0.25 of full scale, where it was 0.1.
- Seen in passing, not looked into: the level leaving was 1.19 of full scale once, where the beep is 0.6. Two pieces sounding at once would do that.

## Limitations and blockers

- **Nobody heard it here, and no real background tab was run.** The numbers say that sound leaves the page; the loudspeaker is the person's.
- If the person pressed "sound" when the pages were in two windows already, this is not the cause of the silence, and the line under the picture will say what is.
- A window that is wholly covered by another can be slowed like a background tab; the worker's clock is the same answer, and that was not tried either.
- The camera and the microphone were not looked at again: they do not run on a timer of the page.
