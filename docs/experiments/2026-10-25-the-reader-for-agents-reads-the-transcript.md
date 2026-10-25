# The reader for agents reads the transcript

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4; the reader for agents (`crates/agent`) and the transcriber (`crates/transcriber`).

## Objective

The transcriber publishes what it reads of a broadcast's voice as the track `speech` of `xmoq/transcript/<prefix>`. Does `xmoq-agent` write those pieces as `speech` lines, the ones said before it came as well as the ones after?

## Expected

Written before the first run, at the top of `interop/transcriber.ts`:

- `xmoq-agent --namespace xmoq/voice/page --transcript xmoq/voice`, which comes after the pieces the live reader got, writes the latest of them, fetched by its Joining FETCH, and then a piece said after it came, as lines `{"type":"speech","number":N,"from":["xmoq","voice","page"],"text":...,"start_us":...,"end_us":...,"read_at":...}` whose text has "relay"; each number once, going up by one.

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless; the own relay, `xmoq-agent` (debug builds) and `xmoq-transcriber` (release, built with the cmake of `mise.toml`) of this tree, with `ggml-tiny.bin`; on `main` at `a75a35cb` with this change. `bun interop/transcriber.ts --port 33120` once, then `mise run interop -- --base-port 33000` with the machine's lock.

## Result

**As expected, in the run alone.**

- The reader's subscription to `speech` was answered at once (`XMOQ_AGENT_SPEECH track_alias=0`).
- It wrote Groups 1792890708048 and 1792890708049 (the two the live reader had, by its FETCH) within 10 ms of each other, 3.1 s and 0.4 s after the transcriber had read them, then 1792890708050 live, 1 ms after the transcriber had read it (`read_at` 1792890715477, `at` 1792890715478). Each was `"text":"Hello from the relay."` from `["xmoq","voice","page"]`, of 1,360 to 1,480 ms.

The full run ended 228 of 229 as expected (518.5 s), the transcriber's pairing with the reader among those that did. The one that did not is `own relay: fetch: own-browser asks for a Group of 8.3 MB the relay holds, reads nothing and gives up; the relay gives the FETCH up`: the relay's writing ended `aborted` where the run waits for `stopped`, both with `CANCELLED` after 62 Objects. This change touches neither the relay nor the page; run alone right after, it ended as expected (`stopped pieces=62 code=0x1`).

## What was not run

- A room: the transcript's token from a room (`sub` in `xmoq/transcript/<prefix>`), and `--transcript` left to the broadcast's namespace, which the interop run, whose transcriber listens above the page's namespace, does not use.
- A reader that connects again and fetches what it missed of the transcript (the rule is the chat's, `history_to_fetch`, whose rows are in `fixtures/chat-history/`).
- A transcriber that comes after the reader, whose SUBSCRIBE the relay holds.
