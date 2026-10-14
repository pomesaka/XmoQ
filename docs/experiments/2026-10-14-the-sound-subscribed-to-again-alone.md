# The sound subscribed to again alone, while the picture goes on

- Date: 2026-10-14
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a session that goes on; what a viewer does when a relay ends the subscription of one track.

## Objective

Behind a narrow link, a viewer that asks no delivery timeout of the video had one of its subscriptions ended by the relay after about 105 s (#241). The watching page followed the end of the video alone (`keepPlaying` and the `ended` of its `play`): had the relay ended the sound's, the picture would have gone on without sound for good. Now the page plays the sound with `keepTrackOnSession` of `@xmoq/media`, which subscribes to it again on the same session when its subscription ends, after 1 s (`DEFAULT_ASK_AGAIN_DELAY_MS`), or after the Retry Interval of a refusal.

The scene: `/watch?cutSound=5` cancels the first subscription of the sound 5 s after it was made (an UNSUBSCRIBE of the page's own; the relay's ending would be a PUBLISH_DONE, which ends the subscription in the same way for the page: `ended` of `playAudioTrack` resolves). The report of the page is read every 100 ms for 14 s.

## Expected, written before the runs

- **The sound comes back 1.0 to 1.3 s after it stopped**: the pause of 1 s, a SUBSCRIBE and its answer (well under 10 ms on one machine), and the first piece through the buffer of 50 ms. About 50 to 65 pieces are not sounded.
- **The picture does not stop**: no time longer than 100 ms over which no frame is shown (a frame every 33 ms; a reading every 100 ms), and one session through the run (`playing (session 1)`).
- The page has subscribed to the sound twice at the end, and it stands at `playing`.

## Environment

macOS 14.4, arm64, loopback; Chromium of playwright-core 1.64.0, headless; the demo (`demo/serve.ts`) from the working copy, ports 45381, 45382, 45384; the own relay with no flag; no proxy. Three runs.

## Result

**The page writes its report every 500 ms**, which the expectation did not take into account: the readings every 100 ms see a change twice a second at most, and the times below are to within half a second.

| Run | No piece sounded for, ms | From, to (ms into the reading) | Longest time with no more frames shown, ms | Frames shown in 14 s | Subscriptions of the sound at the end | Sessions |
|---|---|---|---|---|---|---|
| 1 | 1,451 | 3,964 to 5,415 | 524 | 452 | 2, playing | 1 |
| 2 | 1,448 | 4,037 to 5,485 | 522 | 437 | 2, playing | 1 |
| 3 | 935 | 4,540 to 5,475 | 523 | 438 | 2, playing | 1 |

- **The sound came back, and the picture went on**: the sound was subscribed to a second time and played on the same session, 0.9 to 1.5 s after it stopped (1.0 to 1.3 s expected; within the half second the readings allow). One session throughout.
- The picture never stood still for longer than one report (522 to 524 ms, at 3.4, 8.5 and 11.5 s, not only near the cut), and 437 to 452 frames were shown in 14 s, 31 to 32 a second: as expected, to within what the readings tell.
- An earlier set of three runs, before the readings said when the picture stood still, had the same: 1,451 to 1,554 ms without sound, 2 subscriptions, 1 session.
- **Not run**: the relay ending the sound's subscription itself (PUBLISH_DONE, or a reset of its streams) rather than the page cancelling it. Both end `ended` of `playAudioTrack`, from which `keepTrackOnSession` goes on; the path in the relay is not tried here.

`xmoq-watch` (read, not run): when the audio's subscription ends it ends the playing of the broadcast (`"the audio track ended"`, `crates/watch/src/bin/xmoq-watch/main.rs`). With a room it then connects anew, and the picture stops for that; without a room the viewer ends.
