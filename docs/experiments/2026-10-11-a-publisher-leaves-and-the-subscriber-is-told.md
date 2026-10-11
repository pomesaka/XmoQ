# A publisher leaves: what the relay tells the subscriber, and when

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay

## Objective

A viewing page stayed "playing" for 8 seconds or more while its publisher's line was cut for 8 seconds. Whether the relay tells the subscribers of a track that its publisher is gone, with what, and how long after.

## Environment

The own relay and the Rust client of the working copy this record is part of, debug builds, loopback, macOS. A publisher that sends an Object a second, and one subscriber.

## Commands

A relay, `xmoq-moqt-client --mode publish` with a `send` on its stdin every second, `xmoq-moqt-client --mode subscribe --objects 1000`, and after four Objects one of: `publish-done` on the publisher's stdin, `kill -TERM` of the publisher, `kill -9` of the publisher. The time is from that moment to the end of the subscriber's process.

## Observed results

One run each.

| The publisher | The subscriber got | After |
|---|---|---|
| sends PUBLISH_DONE | `PUBLISH_DONE status_code=0x2 stream_count=4` | 2.0 s |
| is ended with SIGTERM | the same | 8.5 s |
| is ended with SIGKILL | the same | 8.5 s |

- **The relay tells every subscriber**, with TRACK_ENDED (0x2; §10.11: "The track is no longer being published") and the number of streams it opened for the subscription, in all three.
- The 2 seconds after a PUBLISH_DONE are the relay's wait for the last Objects of the track (§10.11).
- **The 8.5 seconds are how long the relay takes to notice a publisher that vanished**: its log has `CONNECTION_ENDED session=0 Lost("connection error: timed out")` and the PUBLISH_DONE right after. A process that is ended closes nothing, so the relay learns of it by its idle timeout alone: 6 seconds (`IDLE_TIMEOUT` in `crates/moqt-io/src/wt.rs`, or `--idle-timeout-ms` of the relay), which begin when the relay's keep-alive goes out, up to 3 seconds (`KEEP_ALIVE`) after the last packet of the publisher. So 6 to 9 seconds.
- **A line that is cut for 8 seconds is shorter than that**: the relay had not counted the publisher as lost when the line came back, and had nothing to tell.

## Limitations and blockers

- The Rust client alone; no page.
- No line was cut: the publisher's process was ended, which is a publisher that does not come back.
- A first attempt took the end of the publisher's stdin for its leaving. The client stays in its session then, and the subscriber was told nothing for 35 seconds, rightly.

## Decision or next action

Nothing in the relay is changed. What a viewer shows while its publisher is silent and not yet counted as lost is the viewer's to decide: the relay says nothing in that time.
