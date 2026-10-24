# The gatherer asks a participant again

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, chat.

## Objective

The gatherer (`xmoq-chat`) let a participant go for the rest of the session when its subscription to the participant's `said` ended or was refused while the namespace stayed announced. It now asks again by the rule of every client here (`xmoq-again`, moved out of `xmoq-watch`), up to `--ask-again-times` times in a row. Does a participant that ends the gatherer's subscription get subscribed to again, and is its next line taken, through both relays?

## Expected

Written before the runs.

- `interop/chat.ts`: the own-rust participant writes `publish-done` on its input (a new line of its `publish-namespace` mode: PUBLISH_DONE `TRACK_ENDED` on the subscription it serves, with the namespace still announced). The relay passes the end on, and the gatherer prints `SUBSCRIPTION_DONE`, then `ASKING_AGAIN ... in_ms=1000`, and a second later subscribes again: the participant prints `SERVING` a second time and the gatherer `SUBSCRIBED` a second time. The participant's next line is taken.
- Through `moqtail`'s relay 0.15.0 (`--restart no`): `b` does this before the late viewer, and its line is numbered 9. The late viewer then expects an End of Unknown Range up to Group 3 and Groups 4 to 9; the pairing is still expected to fail there, at the indicator that relay does not pass on (`matrix.ts`), now with "not 3".
- Through the own relay: `a` does this after the restart, its line is numbered 10, and the room's PUBLISH_DONE counts the two streams the second gatherer opened (`streams=2`).
- Not known before the run: whether `moqtail`'s relay passes a PUBLISH_DONE of its upstream subscription on to the subscriber, and serves a second SUBSCRIBE for the same track from the same session.

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.14; Chromium 156.0.8078.4 of playwright-core; the own relay, `xmoq-chat` and the own-rust client of this tree (debug builds); `moqtail`'s relay 0.15.0. `bun interop/chat.ts --relay own --port 49150` once, then `mise run interop -- --base-port 49100` twice: once on `8508aaaa`, and once after this change was put on `9c5f6c9a` (the gatherer's tokens, PR #304).

## Result

**As expected, through both relays, in each run.**

- Through `moqtail`'s relay: `b` sent PUBLISH_DONE (`PUBLISH_DONE_SENT`); the gatherer printed `SUBSCRIPTION_DONE room="r" name="b" status=0x2`, `NOT_SUBSCRIBED ... why="PUBLISH_DONE"` and `ASKING_AGAIN room="r" name="b" in_ms=1000`; `b` was asked again (`SERVING track_alias=1`), the gatherer printed `SUBSCRIBED` for `b`, and `b`'s next line was numbered 9. So that relay passes the end of its upstream subscription on and serves the second SUBSCRIBE. The run then failed as expected at the late viewer ("told of unknown lines up to: none, not 3"); the viewer got Groups 0 to 5 as before.
- Through the own relay: the same for `a` after the restart, its line numbered 10, and `ROOM_DONE room="r" status=0x2 streams=2`.
- When the participants left at the end (their input closed), each sent PUBLISH_DONE before it withdrew its namespace: the gatherer printed `ASKING_AGAIN` for each, and then `PARTICIPANT_GONE`, and asked none of them again.

Both full runs ended 204 of 205 as expected (469 s and 536 s). The one that did not, both times, was "own relay: watch: an own page publishes a catalog, video and sound on one connection, and the terminal viewer finds and plays them": the page published its broadcast and closed its session, and the viewer printed nothing before the driver's wait for `CATALOG` ran out. The viewer did not reach its subscriptions, where this change acts (the pause before the sound is asked for again). Run alone three times after the second full run (`bun interop/matrix.ts --only "an own page publishes a catalog, video and sound on one connection" --base-port 49300`), it ended as expected each time. Why it fails in the full run was not looked into; the [record of 2026-10-24 on a page that withdraws its chat namespace](2026-10-24-a-page-withdraws-its-chat-namespace.md) has the same pairing fail once in a full run.

## What was not run

- A refusal of the participant's `said` (REQUEST_ERROR), and the pauses after one: the unit tests of `xmoq-again` and `src/said_again.rs` alone.
- The limit of `--ask-again-times`, and a NAMESPACE that comes again after it: the unit tests alone.
- A participant of `@xmoq/chat` (a page) whose subscription ends: the pages give the subscription no way to end it short of withdrawing the namespace.
