# The first picture of a viewer who comes late: fetch the Group, or wait for the next

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: the open question "How a viewer who joins mid-broadcast gets its first picture". Material for it; nothing is decided here.

## Objective

Two of the ways the draft offers (§5.1.3) are in `playVideoTrack`: a Joining FETCH of the latest Group, which the relay answers from its cache, and waiting for the next Group. How long until the first picture with each, how old is that picture, how long until the viewer is where the others are, how many frames are decoded for nothing, and how does the sound begin?

Expected, written before the runs: with the FETCH the first picture comes at once and is old by up to a Group, and the viewer catches up within a fraction of a second, having decoded the Group's start for nothing; waiting takes half a Group on average and up to a whole one, and nothing is decoded for nothing; the sound begins at once either way, so with waiting it is heard before anything is seen.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay | own, Cubic, with its cache of the latest Group |
| Sent | the demo's clock, 640x360, 30 frames a second, about 450 kbit/s; a key frame every 1,000 ms and every 2,000 ms (`/publish?keyint=`). Opus, ten pieces a Group |
| Viewers | one watches already, so that the relay has the track. Then ten come one after the other, each at another moment of a Group: five with the FETCH (`/watch`) and five without (`/watch?join=no`). Buffer 50 ms |
| Link | none, and `spikes/lossy-link/proxy.mjs` with 50 ms each way between the late viewer and the relay |

Times are from the moment both of the viewer's tracks were subscribed to (14 ms after the page began on loopback, about 735 ms with 50 ms each way, the same for both ways).

## Result

Five viewers each; the smallest and the largest.

| Key frame every | Link | Way | First picture after, ms | How old that picture was, ms (it settles at) | Caught up after, ms | Frames decoded and not shown | Dropped by the receiver |
|---|---|---|---|---|---|---|---|
| 1 s | none | FETCH | 9 to 60 | 31 to 70 (about 36) | 0 | 5 to 24 | none |
| 1 s | none | wait | 695 to 1,043 | 30 to 35 | 0 | 0 to 1 | 19 to 30 as `no-key-frame` |
| 1 s | 50 ms | FETCH | 0 to 36 | 237 to 1,060 (about 95) | 17 to 84 | 2 to 14 | twice 4 as `gap` and 22 to 24 as `late` |
| 1 s | 50 ms | wait | 0 to 971 | 91 to 118 | 0 | 1 to 2 | 1 to 30 as `no-key-frame` |
| 2 s | none | FETCH | 9 to 60 | 48 to 75 (about 43) | 0 to 50 | 7 to 59 | none |
| 2 s | none | wait | 294 to 1,959 | 44 to 47 | 0 | 1 to 2 | 7 to 26 as `no-key-frame`; twice 33 more as `overflow` |
| 2 s | 50 ms | FETCH | 3 to 83 | 732 to 1,576 (about 90) | 99 to 250 | 18 to 44 | none |
| 2 s | 50 ms | wait | 105 to 1,570 | 87 to 97 | 0 | 1 to 2 | 2 to 16 as `no-key-frame`; three times 33 more as `overflow` |

- **The expectations held.** With the FETCH the first picture came within 83 ms in all twenty runs; waiting took up to a whole Group (1,043 ms and 1,959 ms).
- **What the FETCH costs** is the start of the Group, decoded and not shown: up to 24 frames with a Group of a second, up to 59 with one of two. On loopback the first picture shown was already nearly the newest (the repaint shows the newest frame that is due, and the decoder was through the Group by then). **Over a link the first picture was old**, by up to 1.6 s with a Group of two seconds, and the viewer was where the others are a quarter of a second later at most. Until then the picture runs fast.
- **Twice the FETCH came too late** (50 ms each way, the viewer came 27 frames into a Group of 30): the next Group began by the subscription before the fetched one was through, and the receiver dropped the rest of what was fetched (`gap` 4, `late` 22 to 24). The viewer had its first picture after 18 and 34 ms all the same, a second old.
- **A viewer that waits holds what comes before the key frame and drops it** (`no-key-frame`), and with a Group of two seconds it reached the limit of 32 held Objects five times of ten (`overflow`, 33). Nothing went wrong by it; the frames could not be played either way.
- **The sound begins at once either way**: the first piece came 1 to 69 ms after its SUBSCRIBE in all forty runs, and none was too late but one. A piece of Opus needs no key frame, and the relay sends the pieces of the Group that is under way. So a viewer that waits hears the broadcast for up to a Group before it sees it; with the FETCH, picture and sound begin together, the picture behind the sound for the fraction of a second it takes to catch up.

## Not done

- The third way of §5.1.3, asking the publisher for a new Group (NEW_GROUP_REQUEST, which needs REQUEST_UPDATE): nothing here has it. With one viewer the demo's publisher begins a Group when the relay subscribes to it, which is that way in effect, and why a first viewer was kept watching in these runs.
- A video that takes more room: here a Group is about 56 KB a second, and fetching it is nothing to the link. At 2.5 Mbit/s a Group of two seconds is 600 KB to fetch before the picture is live.
- Loss, a narrow link, more than one late viewer at once, a relay without a cache (the FETCH is refused and the viewer waits, as the README of `@xmoq/media` has).
- How the catching up looks, and whether sound before picture is worse than silence. Nobody looked or listened.
