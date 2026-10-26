# An agent in a meeting

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, meetings, the reader for agents (`crates/agent`, `crates/viewer`). The meeting is **proposed** (`docs/scale-and-meetings.md`, Part 2); how its participants find each other is undecided, and this is the side that reads the relay's namespaces.

## Objective

In a meeting every participant publishes a broadcast of its own under `xmoq/meet/<room>/<id>`. Does `xmoq-agent --meeting <room>` learn who is there from one `SUBSCRIBE_NAMESPACE`, give the picture of the one `/frame <id>` names, refuse a picture of nobody, and write a participant who goes as left?

## Expected

Written before the first run, and before the reader had `--meeting`, at the top of `interop/agent-meeting.ts`:

- The reader writes `{"type":"participant","state":"joined","id":"p1"}` and the same for `p2`, in either order, and no other participant.
- `/frame` alone is answered with the error line `frame: nobody has spoken; /frame <id>, and there are p1, p2`.
- `/frame p1` is answered with a `frame` line of `"id":"p1"`, and a PNG of 640x360 that a page decodes to the grey and the green square it painted, as `agent-frames.ts` checks them.
- `/frame p2` then, with a `frame` line of `"id":"p2"` and such a PNG.
- `/frame p3`: the error `frame: p3 is not in the meeting, and there are p1, p2`.
- The page of p2 closes its session: the reader writes `{"type":"participant","state":"left","id":"p2"}`, and `/frame p2` after it is answered with `frame: p2 is not in the meeting, and there are p1`.
- events.jsonl has the same lines as stdout.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 of playwright-core, headless; the own relay and `xmoq-agent` of this tree (debug builds), on `main` at `wwkvtops` (`xmoq_viewer::meeting` in) with this change for `mise run interop -- --only "agent: xmoq-agent --meeting" --base-port 31500`, then on `pwxrmsky` for the whole matrix with the machine's lock.

## Result

**As expected, in the first run** (2.1 s). The reader wrote 8 lines: p1 and p2 joined, the error of `/frame` alone, a `frame` line of p1 and one of p2, the error of `/frame p3`, p2 left, and the error of `/frame p2`. The page decoded both PNGs to the grey 67, 67, 67 at (20, 20) and 40 green pixels on the row 315. `events.jsonl` had the same 8 lines.

The change was then put on `main` at `pwxrmsky` (the reader's chat on `xmoq_viewer::presence`, and the room's transcript prefix). There the whole matrix ended 234 of 234 as expected (516.3 s), this pairing among them.

## What was not run

- `/frame` alone of the one who spoke last: no transcriber runs here, so nobody's voice is read. Run since in [who speaks in a meeting](2026-10-25-who-speaks-in-a-meeting.md).
- The video let go of after `--meeting-video-linger-ms`, a participant over `--meeting-most`, and a meeting over a GOAWAY or a new session: the table tests of `xmoq_viewer::meeting` and `xmoq_viewer::presence` alone.
- A meeting through a room (`--room`): not built, waiting for the decision on how participants find each other. A meeting through `moqtail`'s relay.
- How long the first picture of a participant takes (a catalog, a subscription, and a key frame): not measured.
