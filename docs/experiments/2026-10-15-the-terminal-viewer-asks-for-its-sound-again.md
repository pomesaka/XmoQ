# The terminal viewer asks for its sound again

- Date: 2026-10-15
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a session that goes on; what a viewer does when a relay ends the subscription of one track

## Objective

`xmoq-watch` ended the playing of a broadcast when the sound's subscription ended ("the audio track ended"): with a room it connected anew and the picture stopped for that, and without one the viewer ended. A relay ends the subscription of the one track that falls behind. Now the viewer asks for the sound again on the same session, by the rule and the values of the watching page ([the page's record](2026-10-14-the-sound-subscribed-to-again-alone.md)), and the picture goes on. `--cut-sound-s N`, for measuring, cancels the sound's subscription once N s after it was made, as the page's `?cutSound=N` does.

## Expected, written before the run

- The log has `AUDIO_ENDED cut` at about 5 s, then `AUDIO_AGAIN` about 1.0 s later.
- One session: one `CATALOG` line and one `AV_PLAYED`, "the viewer was ended".
- The picture goes on: about 30 frames a second, 350 or more in 14 s.
- The sound: about 50 packets a second but for about 1 s, 550 or more.

## Environment

macOS 14.4, arm64, loopback. The demo (`demo/serve.ts`, ports 48081, 48082, 48084) from the working copy, the own relay with no flag; the publishing page in headless Chromium (playwright-core of `interop/`); `target/debug/xmoq-watch --room http://127.0.0.1:48084/rooms/<room> --seconds 14 --sound off --out /dev/null --medium direct --fit none --cut-sound-s 5 --log <file>`. One run.

## Result

- The log: `CATALOG` once, `AUDIO_ENDED cut; asking again in 1000 ms`, `AUDIO_AGAIN track_alias=3`, and one `AV_PLAYED why="the viewer was ended" seconds=14.0`.
- The relay's log: the sound's request stream reset and stopped with `0x1`, the relay's SUBSCRIBE to the publisher cancelled, then a new SUBSCRIBE on the viewer's stream 5 and its SUBSCRIBE_OK, on the same session (session 1 throughout).
- The picture: 422 frames came and 422 were drawn, 30.3 a second, none left out.
- The sound: 649 packets in 14 s, about 50 fewer than 50 a second would give, so about 1 s without; 96 underruns, which `--sound off` with a buffer of 50 ms has as well (not compared with a run without the cut).
- The log has no times, so when within the run the sound stopped and came back is read from the packets only.
- Not run: the relay ending the sound's subscription itself (PUBLISH_DONE or a reset); a refusal on the way back, and its Retry Interval.
