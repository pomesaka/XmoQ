# The reader says its counts before it ends

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 5; the reader for agents (`crates/agent`).

## Objective

The pairing `own relay: agent: xmoq-agent takes no picture of a page's broadcast unasked …` (`interop/agent-frames.ts`) failed once in CI (the run of #339, https://github.com/pomesaka/XmoQ/actions/runs/40152835104/job/121047774891): all three `frame` lines were there, and the last check, `the reader's last counts: none`, found no `XMOQ_AGENT_FRAMES` line. That line is written by the thread of the pictures (`frames.rs`) when its queue ends. At SIGTERM `main` returned without waiting for that thread, and the process could end before the thread wrote it (read in the code, not seen).

The reader now ends the thread and waits for it, 2 s at the most (`Pictures::end`), on every way out of `run`.

## Expected

Written before the runs: the pairing alone (`--only "agent: xmoq-agent takes no picture"`), 10 times before the change and 10 times after, ends as expected each time after; before, it may fail now and then as in CI.

## Environment

macOS 14.4, arm64, loopback, the machine otherwise as it was (other sessions working); Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless; this tree on `main` at `5f5252ac`, without and with this change; `mise run interop -- --base-port 33000 --only ...` 10 times in a row each.

## Result

- Before: 10 of 10 as expected (7.2 to 8.2 s). **The failure of CI was not seen here.**
- After: 10 of 10 as expected (7.3 to 9.3 s).

So these runs do not show the change fixing the failure: the race did not happen on this machine before either. What they show is that waiting for the thread changes nothing that the pairing checks, and adds no time it notices.

## What was not run

- A run that loses the line before the change (on a machine as loaded as CI's runner).
- The full run.
