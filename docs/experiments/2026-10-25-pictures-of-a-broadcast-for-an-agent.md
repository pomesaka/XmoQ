# Pictures of a broadcast for an agent

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4; the reader for agents (`crates/agent`), its pictures (`crates/picture`).

## Objective

An agent reads a broadcast's picture as a PNG file it asks for. Does `xmoq-agent`, through the own relay, take no picture unasked, answer `/frame` with a PNG of the page's video and a `frame` line, and keep no more files than `--frames-kept`, also over the sessions of a reader that moves to another relay?

## Expected

Written before the first run, at the top of `interop/agent-frames.ts`, with `--frames-kept 1` and one relay then:

- The reader finds the video track in the catalog and subscribes to it: `XMOQ_AGENT_VIDEO track=video` on its stderr.
- Unasked, it takes no picture: 3 s after that line, no `frame` line, and no file in `<out>/frames`.
- `/frame` is answered within 10 s with a `frame` line, `"asked":"command"`, 640x360, whose `path` is in `<out>/frames` and ends in `.png`. The file is a PNG of 640x360 (its IHDR). A page decodes it: the pixel at (20, 20) is the grey the page paints (`#444`, 68 each) within 16, and the row 315 has 30 or more pixels of the green square (`#3a7`: green at least 50 over red), which is 40 wide.
- A second `/frame`, 2 s later: another `path`, a later `timestamp_us`, and with `--frames-kept 1` the first file is removed and the second is there.
- At its end (SIGTERM), `XMOQ_AGENT_FRAMES ... taken=2 ...` on stderr.

Added once the reader kept one `Frames` for its run (`begin_again`, #333), written before the first run of this part. Two own relays, as in `watch-rust.ts --watch goaway`: the run's, with `--redirect-uri` naming the second, and a page publishing through each; `--frames-kept 2` in place of 1, so that the two pictures above are both kept:

- The run's relay is sent SIGTERM and sends GOAWAY naming the second; the reader moves there, and says `XMOQ_AGENT_VIDEO track=video` a second time.
- A third `/frame` is answered on the second relay with a picture of that page's broadcast, as the first is checked, under a name none of the two before had; the first file (of the first session) is removed for it, and the second and the third are there.
- At its end, the last `XMOQ_AGENT_FRAMES` says `taken=3`: the counts go on from one session to the next.

WHY a GOAWAY, and not a relay stopped and started again as in `video.ts`: with `--url` the reader connects again only where it was moved (`follows` in `crates/agent/src/bin/xmoq-agent/main.rs` is for a room); a session that ends or fails to connect ends it.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 of playwright-core, headless; two own relays and `xmoq-agent` of this tree (debug builds). `mise run interop -- --only "agent:" --base-port 31500`: twice with one relay on `main` at `347d0ebe` (`xmoq-agent` in); once with two relays on `main` at `347d0ebe`, before #333; twice with two relays on `main` at `b84279e3` (#333 in). Then the whole matrix with the machine's lock, on `b84279e3`.

## Result

**One relay: as expected, in both runs.**

| | First run | Second run |
|---|---|---|
| The pixel at (20, 20), as the page decoded the PNG | 67, 67, 67 | 67, 67, 67 |
| Pixels of the green square on the row 315 | 40 | 40 |
| The second picture, of | Group 1792887246288, Object 12 | Group 1792887361995, Object 16 |
| `Frames`' counts at the end | given 172, decoded 43, passed over 121, taken 2 | given 190, decoded 46, passed over 135, taken 2 |
| The pairing | 6.9 s | 8.3 s |

Between the runs only the driver changed: how it hands the PNG to the page (a script as a string, for the type check).

The counts show what `Frames` decodes: about a quarter of the Objects that came, those of the Group that was current at each `/frame`, from its first up to the latest; the rest were let go undecoded. No `left_out`, `no_picture` or `not_written`.

**Two relays, before #333: failed where it was to.** The reader moved and answered the third `/frame` on the second relay, and the first file was not removed: `<out>/frames` had `s1-000001-...`, `s1-000002-...` and `s2-000001-...`. A session's `Frames` began anew then, and `--frames-kept` held for each session alone.

**Two relays, with #333: the first run failed, and the second ended as expected after a change of the driver.**

- First run: the third `/frame` was written as soon as the reader said `XMOQ_AGENT_VIDEO` on the second relay, and was answered at once with the error `frame: waiting for the chat`: while a session waits for something, the chat here, the reader answers every line of stdin, `/frame` among them, with such an error (`Reader::unless_ended`). The driver now waits for the second `XMOQ_AGENT_CHAT` line as well. Nothing was expected of a `/frame` sent before then; it was not written down.
- Second run: as expected. The third picture had the grey 67, 67, 67 and 40 green pixels; the files after it were the second and the third, with no `s<n>-` before their names; `XMOQ_AGENT_FRAMES given=141 left_out=0 decoded=37 no_picture=0 too_large=0 passed_over=103 taken=3 not_written=0` (6.7 s).

The whole matrix, on `b84279e3` with this change, ended 226 of 226 as expected (482.1 s), this pairing among them (6.2 s, `taken=3`).

## What was not measured or run

- How long `/frame` waits for its answer: the driver does not time it, and the reader's `at` is not set against the moment of the command.
- `--frame-every-ms`, `--frame-longest`, a `/frame` that gets no picture within 10 s, and a picture of more than 1280 pixels a side (the page sends 640x360): the table tests of `xmoq-picture` alone.
- A reader of a room that follows a broadcast that ends and begins again: the interop runs have no room.
- The publisher of moqlivemock, and `moqtail`'s relay.
