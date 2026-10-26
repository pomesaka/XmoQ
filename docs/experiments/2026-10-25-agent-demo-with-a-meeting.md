# agent-demo with a meeting

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, meetings; the reader for agents.

## Objective

`scripts/agent-demo.sh start --meeting N` opens N participants of the demo's meeting (`/meet`) in place of the publishing page. Each is a headless Chromium of its own, whose microphone is the voice. Page *i* has 2 *i* s of silence before the voice and 2 (N − 1 − *i*) + 1.5 s after it, so each begins 2 s after the one before and every round is as long. The reader is started with `--meeting demo`. Can a person who starts it read the meeting through the reader, and does `stop` leave nothing?

## Expected

Written before the run:

- `start --meeting 3 --transcribe-language en` ends by printing the 3 participants' ids, a `/frame <id>` line and how to find the ids in `events.jsonl`.
- `events.jsonl` has a `participant` `joined` line for each of the 3. Within 60 s it has `speech` lines from each one's `["xmoq","meet","demo",<id>]`, about 2 s apart in `start_us` in the order the pages were opened.
- `/frame <id of the 2nd>` gives a `frame` line of that id, and a PNG with that id painted on it.
- `stop` ends the 3 pages, the reader, the holder of the pipe and the demo. It removes `pids`, and no process of them is left. No directory of a voice with silence is left.
- Without `--meeting` the reader still writes a `speech` line from `["xmoq","demo"]`.

## Environment

macOS 14.4, arm64, loopback. Bun 1.3.12, and Chromium 156.0.8078.4 of playwright-core, headless. The script's ports were 33443, 33080 and 33787, with `fixtures/speech/hello-from-the-relay.wav`. The run was on `main` at `25083853` with this change, built anew after a `cargo clean`.

## Result

**As expected.**

```
the reader is in the meeting of http://127.0.0.1:33787/rooms/demo as claude, with 3 participants: a8ad3f13f27736e6 18f61e97e8647707 6026463c2f0833e5
{"id":"a8ad3f13f27736e6","state":"joined","type":"participant"}   (and the 2 others)
"from":["xmoq","meet","demo","a8ad3f13f27736e6"]  "start_us":1792914675645000  "text":"Hello from the relay."
"from":["xmoq","meet","demo","18f61e97e8647707"]  "start_us":1792914678366000  "text":"Hello from the relay."
"from":["xmoq","meet","demo","6026463c2f0833e5"]  "start_us":1792914681510000  "text":"Hello from the relay."
"from":["xmoq","meet","demo","a8ad3f13f27736e6"]  "start_us":1792914682245000  …
{"asked":"command",…,"height":180,"id":"18f61e97e8647707",…,"type":"frame","width":320}
```

The voices came 2.7 s and 3.1 s apart, and each one's again 6.6 s later: the round of 4 s of silence and the voice. The PNG showed `18f61e97e8647707` and a clock. `stop` printed `stopped:` for the 6 processes, `pids` was gone, `pgrep` found none of them, and no `xmoq-open-publish-*` directory was left. Without `--meeting`, the reader wrote `"from":["xmoq","demo"]`, `"Hello from the relay."` 2 s after the start, and `stop` ended its 4 processes.

## What was not run

- `--meeting` with `--auth`: the script has no flag for it.
- More than 3 participants, and a person who opens `/meet` beside them.
