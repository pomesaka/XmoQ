# A meeting of pages

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, meetings, the page's side (`packages/meeting`). How participants find each other (SUBSCRIBE_NAMESPACE), where the limit of 49 holds (the room, and 48 others in a page) and what each one receives at first (the small video of 8 as they came, the sound of all) were decided on 2026-10-25 ([design](../scale-and-meetings.md)).

## Objective

Three pages each publish a broadcast under `xmoq/meet/r/<id>` and take part with `createMeeting` of `@xmoq/meeting`. Does each receive the others' sound, the video of the first who came up to its limit, and let go of one who leaves, giving its place among the watched to the next?

## Expected

Written before the first run, at the top of `interop/meeting-pages.ts`:

- p1 publishes and attends, then p2, then p3, each when the one before has. p1 is told `came p2` before `came p3`.
- p1 (one video) receives the video of p2 alone, and the sound of p2 and p3: frames of p2 and none of p3, pieces of sound of both. p2 and p3 receive the video and the sound of the two others. Nobody is told of itself.
- p2 closes its session. p1 and p3 are told `left p2`, and `stopped p2 video` and `stopped p2 audio`; p1 then `receiving p3 video`, and frames of p3 come to it.
- After that nothing more of p2 comes to p1 or p3: its counts of frames and of sound stay the same over a second.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 of playwright-core, headless; the own relay of this tree (debug build) and `moqtail`'s relay of `interop/peers/`, on `main` at `ptxkzvmm` with this change, `mise run interop -- --only "meeting: three pages" --base-port 31500`.

## Result

**As expected through the own relay, in the first run** (3.6 s), and again in a second (3.2 s). p1 was told `came p2` then `came p3`, received the video of p2 alone and the sound of both, and nobody was told of itself. When p2 closed its session, p1 and p3 were told `left p2` and stopped its video and sound, and p1 received the video of p3. At the end of the second run p1 had 15 frames of p2 and 32 of p3, and 26 and 61 pieces of their sound, and the counts of p2 stayed the same over the last second at p1 and p3.

The whole matrix then ended 239 of 239 as expected (512.9 s, `--base-port 31500`), this pairing among them.

With p1 watching one video more than it is given (`watchedOf` changed to keep `most + 1`), the run failed as it should: `p1 had frames of p3 while it watched p2`.

**Through moqtail's relay it failed** (21.1 s): every page was told of the others, but no catalog came. moqtail's relay counted the Joining FETCH of each catalog from {0, 0} to {0, 1} and asked the page for Group 0, where the page numbers its Groups by the clock (its SUBSCRIBE_OK had Largest Object {1792907901367, 0}); the page answered with an empty stream. This is the same as in the [GOAWAY runs](2026-10-24-goaway-and-moqtail.md), moqtail's and not the page's, and the pairing is not in the matrix through moqtail's relay.

## What was not run

- Through a room, with its tokens: the room's `join-meeting` is built apart.
- A meeting over a GOAWAY or a new session, and more than 48 others: the table tests of `@xmoq/meeting` alone.
- A catalog with several versions of the video: the pages publish one, so the smallest is that one.
- Playing what is received: the pages count frames and pieces of sound and close them.
