# agent-demo starts and stops

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 5; the reader for agents (`crates/agent`).

## Objective

`scripts/agent-demo.sh` (`mise run agent-demo`) starts the demo on other ports, the reader in its room and the publishing page in a headless Chromium, writes the four processes into `<out>/pids`, and `stop` ends them. Checked: that `start` returns once the broadcast began, that the reader reads the room, that `stop` leaves none of the four nor what they started, that a second `start` on the same `--out` refuses, and that `stop` leaves alone a PID that is not one, or is now another process's.

## Expected

Written before the runs: `start` ends with the reader's `broadcast` `began` line; `speech` lines follow from the voice of `fixtures/speech/hello-from-the-relay.wav`; after `stop`, `ps` finds none of the PIDs of the four and of their children (the relay, the rooms with workerd, the gatherer, the transcriber, Chromium), and `<out>/pids` is gone.

## Environment

macOS 14.4, arm64, bash 3.2.57; Bun 1.3.14 (mise); Chromium 156.0.8078.4 of playwright-core 1.64.0; `main` at `76c58169` with this change; the ports 33443, 33080 and 33787; `--transcribe-language en`.

## Result

**As expected**, after one fix:

- The first `start`, run as `mise run agent-demo -- start ... | tail`, did not return: the process holding the pipe open had this script's stderr, and `tail` waited for its end. The processes themselves had started (the broadcast began 1 s after the page opened, and a `speech` line came every 15 s). The holder now has its stdin and stderr on `/dev/null`. `stop` on that run ended the four and their six children (`ps -p` found none of the ten PIDs).
- The second `start`, the same way, returned with `captions: the transcriber reads xmoq/demo in en` and the commands. `hello from the script` and `/frame` into `<out>/say` gave a `chat` and a `frame` line. A second `start` on the same `--out` refused, exit 1. `stop` ended the four; `ps -p` found none of them nor of their six children (relay, rooms, workerd, gatherer, transcriber, Chromium); `<out>/pids` was removed.
- `stop` on a `pids` of `0`, `abc`, and PID 1 with another start time and command: `left alone: not a PID` for the first two, `already gone (or its PID is another's now)` for PID 1; exit 0, nothing signalled.

## What was not run

- `--tone`, and `start` failing halfway (its `stop` of what was started).
- Linux.
