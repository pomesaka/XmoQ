# The audio and video demo: what it does, and where it ends

What the demo (`mise run demo`, `demo/`) was seen to do by 2026-10-12, and what it was not. Every number is from a run that is recorded under `docs/experiments/`; the record has how it was got. **All of it is one machine**: publisher, relay and viewers on loopback, and for a link a proxy that delays, drops and limits (`spikes/lossy-link`). Nothing here was run over a real network, and almost nothing was looked at or listened to by a person.

## What it is

A page encodes a picture (a clock it paints, or a camera) as H.264, 640x360 at 30 frames a second with a key frame a second, and a tone (or a microphone) as Opus, 48,000 Hz mono, and publishes both through the own relay with an MSF catalog. Other pages find the tracks in the catalog and play picture and sound by one clock. A room (a Durable Object in Miniflare) says where the broadcast is and carries a chat. The parts are `@xmoq/moqt`, `@xmoq/loc`, `@xmoq/msf` and `@xmoq/media`; the demo's pages draw, count and show.

## What was measured

| What | What was seen | Record | Not measured |
|---|---|---|---|
| Rate | 30 frames a second shown and 50 pieces of sound a second to the loudspeaker; `bun demo/check.ts` checks both, and that what leaves is no silence | [background tab](experiments/2026-10-11-demo-in-a-background-tab.md) | Longer than a minute; CPU of the watching page |
| Delay on one machine | Nearly all of it is the buffer: a frame is shown about 9 ms after it was made with no buffer, 39 to 47 ms with 50 ms (the default), about 141 ms with 150 ms | [delay](experiments/2026-10-11-where-the-delay-of-the-demo-comes-from.md) | From the camera to the encoder, and from the repaint to the screen |
| Picture and sound together | One clock and one buffer for both (`createPlayout`). The picture's mark came 5 to 38 ms after the sound's; a frame is painted every 33 ms | [sync](experiments/2026-10-11-demo-sync-and-delay.md), [tone](experiments/2026-10-11-demo-in-firefox-and-webkit.md) | **Nobody judged the sync by eye and ear** |
| Sound without crackle | 0 or 1 steps in 4 s of what leaves for the loudspeaker, where it was about 100 a second before the pieces were joined | [background tab](experiments/2026-10-11-demo-in-a-background-tab.md) | **Nobody has listened since it was mended.** A real microphone |
| A viewer who comes late | With the Joining FETCH of the latest Group the first picture within 83 ms, up to a Group old over a link, caught up within 0.25 s; waiting for the next key frame takes up to a Group. The sound begins at once either way | [first picture](experiments/2026-10-12-the-first-picture-of-a-viewer-who-comes-late-fetch-or-wait.md) | A video of more than 450 kbit/s; how the catching up looks |
| A session that ends | The relay stopped and started anew: both pages connect again and go on. A link cut for 12 s: both say so after 6 s and play again within 4 s of its return | [reconnect](experiments/2026-10-12-pages-connect-again-when-the-relay-comes-back.md) | What was said meanwhile is not sent later; no limit on the tries |
| Loss with delay (50 ms each way) | Decided by the relay's congestion control. Cubic, 2.5 Mbit/s, 1% loss: the picture falls behind for good (655, then 1,145 ms; 43 of 600 frames shown). BBR: 55 ms and 595 of 600 at 1%, 128 ms and 520 of 600 at 5%. The sound plays with both | [BBR](experiments/2026-10-11-the-demo-over-a-lossy-link-with-bbr.md), [Rust viewer](experiments/2026-10-12-video-over-a-link-that-delays-and-loses.md) | A link shared with other traffic; real loss, which comes in bursts |
| A link with less room than the video | BBR is no better there and mostly worse. With Cubic the picture falls further behind through the run; with the relay's `--delivery-patience-ms 500` (an experiment, off by default) it stays about half a second behind and the page shows more frames, not fewer (444 against 327 of 600 at 2.5 Mbit/s, 347 against 128 at 2 Mbit/s) | [Cubic and BBR](experiments/2026-10-12-cubic-and-bbr-over-a-link-with-too-little-room.md), [page](experiments/2026-10-12-a-page-behind-a-link-with-too-little-room-and-a-relay-that-gives-groups-up.md) | BBR with the page; a picture judged by eye |
| Two versions of the video (a trial, `?ladder=1` and `?abr=1`) | A viewer goes to a 320x180 version 1 to 2 s after its link gets narrow and stays 60 to 75 ms behind, showing 382 to 407 of 450 frames at 2 Mbit/s; it is back on the large one 16 to 23 s after the link has room. A second viewer with room is not touched | [two versions](experiments/2026-10-12-two-versions-of-the-video-and-a-viewer-that-changes-between-them.md) | More than two viewers; a judge that estimates the link, not one that tries |
| A buffer that follows the link (a trial, `?buffer=auto`) | Settles at 40 ms with no loss; at 1% loss it moves between 110 and 140 ms and leaves 6 to 10 steps in the sound where a fixed 150 ms leaves 1 | [delay](experiments/2026-10-11-where-the-delay-of-the-demo-comes-from.md) | Not the default |
| The publishing page in a background tab | It sent a frame a second, and a viewer had no sound. Its timers are a worker's now | [background tab](experiments/2026-10-11-demo-in-a-background-tab.md) | **Not seen in a real background tab**: Playwright keeps every page visible |
| Browsers | Chromium 156. Playwright's Firefox 157 publishes and watches, with everything above | [Firefox and WebKit](experiments/2026-10-11-demo-in-firefox-and-webkit.md) | WebKit (Playwright's build did not start here); a shipped Firefox or Safari; a phone |
| Many viewers | One video track of 513 kbit/s reached 1,000 viewers of the Rust client on one Mac whole | [viewers](experiments/2026-10-11-one-video-track-to-many-viewers.md) | Pages as the viewers; more than one machine |

## What later records took back

- **"The sound falls behind on a link with delay and loss", and "ten pieces on one stream prevent it"** ([lossy link](experiments/2026-10-11-demo-over-a-lossy-link.md)): withdrawn. The sound was not late. Chromium's decoder gave the pieces times of its own count, and each piece dropped as late moved all later ones 20 ms; the player now gives each piece the time of its chunk and puts late pieces in order ([cause](experiments/2026-10-11-sound-falls-behind-on-a-link-with-delay-and-loss.md)).
- **"BBR keeps the picture live"** holds for loss, and not for a link with too little room, where it is worse than Cubic (the row above). The relay's default is Cubic; which to use is undecided.
- **Every number of the sound's lead before 2026-10-12 is 11 ms off**: the demo's tone carried times 11 ms ahead in Chromium (about 100 ms in Firefox), which was the demo's fault and is mended ([tone](experiments/2026-10-11-demo-in-firefox-and-webkit.md)). With it, a buffer of 150 ms held a frame for 133 ms; now for 148. The delays in the table are from before.
- **A later Group of video that waits for the one before it** was tried and not taken: no more frames at 1% loss, fewer at 5% ([record](experiments/2026-10-12-a-later-group-of-video-waits-for-the-one-before-it.md)).

## Where it ends

- **One machine.** No run over a real network, between two machines, or through a NAT; the hosting is undecided.
- **Nobody has watched since 2026-10-11.** That day a person saw one viewer play a publisher's picture smoothly and heard the sound, with crackles, before the seams were mended; since then every statement about picture and sound is a count read from the page.
- **Every value is provisional**: the buffer (50 ms), the sound's least wait (30 ms), what the automatic buffer and the change of versions go by, the pause before connecting again (2 s). `packages/media/README.md` has each with where it is from.
- **A frame that does not come is not concealed.** The end of a Group that comes after the next has begun is dropped; a piece of sound that is missing leaves a gap.
- **Nothing thins the video for a slow viewer by default.** The relay's patience and the two versions are trials behind flags.
- **The camera and the microphone ran only as Chromium's fake devices.**
- **No authorization worth the name**: the relay checks a token of the room, and anybody who asks the room gets one.
- Firefox's Opus encoder gives its chunks times 6.5 ms early; left as it is.
