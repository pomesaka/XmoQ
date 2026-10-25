# A page of a meeting writes for Claude Code

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, an agent in a meeting: without the CLI, the meeting's page (`/meet`) writes what happens into a folder an agent on the same machine reads, and does what the agent writes into it.

## Objective

Does a page of a meeting, given a folder as the File System Access API gives one, write the lines `xmoq-agent` writes, and a PNG of a participant when its `inbox.txt` asks for one?

**Loopback, one machine.** The folder is one of the page's own storage (OPFS), which the API hands as it hands the one a person chooses, with no picker to click.

## Expected

Written before the first run, in the head of `interop/meeting-pages.ts`:

- p1's `events.jsonl` has `participant` lines, `joined` for p2 and then p3; a `frame` line with `id` p2, `asked` command and a `path` under `frames/`, whose file is a PNG (its bytes 1 to 3 are "PNG"); an `error` `not said: this page has no chat` for `hello` (the page there has no chat); and an `error` `/frame p3: no picture of p3 came`, since p1 receives the video of one alone.

## Environment

macOS 14.4, arm64, Chromium of Playwright (156.0.8078.4), headless; the own relay (debug build) on 31500. `mise run interop -- --base-port 31500 --only meeting`.

## Result

**As expected, in the first run.** The four runs with `meeting` in their names passed (18.4 s). p1 wrote five lines; the picture of p2 was at `frames/1792925652972/000001-g1792925653204-o22.png`, 6,500 bytes, a PNG: Group and Object are those of the frame drawn last, as `playVideoTrack` hands them on (`FrameOrigin`).

With `createMeeting` handing `onFrame` no origin (`options.onFrame(id, frame, undefined)`), the run failed as it should: p1 wrote `/frame p2: where its picture came from was not kept` and no `frame` line.

`packages/meeting/test/agent-lines.test.ts` reads `fixtures/agent-lines/lines.tsv`, which `crates/agent` reads as well (since PR #402, which has its `frame` line take its `path` from `--out`); `agent-inbox.test.ts` reads `fixtures/agent-lines/inbox.tsv`, which the CLI's `input.rs` reads too; `agent-bridge.test.ts` what a line of the inbox does.

## Not looked at

- The folder a person chooses, and the permission to write in it again: the picker and Chromium's prompts are windows of the browser, which a script does not click. To be looked at by a person, in Chromium, with the demo:
  1. `mise run demo`, and open `http://localhost:8080/meet` (in two tabs, or two browsers, so that there is somebody to see).
  2. Make `~/Documents/xmoq`. Press **link a folder for Claude Code**, and choose it. Write down what the page then says beside the buttons.
  3. Press **call Claude Code**. Write down what the page says, and check that `~/Documents/xmoq/demo/<id>/events.jsonl` has `participant` lines.
  4. `echo '/frame <the other tab's id>' >> ~/Documents/xmoq/demo/<id>/inbox.txt` and `echo 'hello' >> …/inbox.txt`: within a second or two a `frame` line and a PNG under `frames/`, and `hello` in the chat of both tabs.
  5. Close the tab and open `/meet` again. Write down what the page says before anything is pressed, then press **call Claude Code**, and write down each window Chromium shows and its choices (whether it offers to allow on every visit).
  6. Choose to allow on every visit, if offered; close and open again, and write down whether **call Claude Code** then asks anything.
- A `speech` line from the meeting's transcript: no transcriber runs in `meeting-pages.ts`.
- The demo's page itself in a browser with this change: type-checked and bundled.
