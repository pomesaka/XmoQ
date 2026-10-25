# Objects before their SUBSCRIBE_OK

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the MoQT layer of both implementations; Phase 4, the chat.

## Objective

On `main` after #375, CI failed the pairing "own relay: watch: two terminal viewers chat on MoQ tracks through the gatherer ..." with `viewer:wa exited with code 0 before printing /^CHAT 0 wa: a1$/`. In the relay's log, wa's subscription to the room's lines was answered (`SUBSCRIBE_OK track_alias=3`) as the gatherer published line 0. The relay then opened the stream of line 0 to wa at once. wa printed `CHAT_TRACKS_LINES track_alias=3` after that and never printed the line.

Read in the code, not reproduced:
- The terminal viewer takes the SUBSCRIBE_OK and the Objects in two arms of one `select!`. An Object read first, while its alias was not yet known, was dropped.
- `@xmoq/moqt` holds such Objects while a SUBSCRIBE waits for its answer.
- The gatherer and the transcriber kept the last 64 of them, by another rule.

§11.4.2 lets a receiver abandon such a stream or "buffer it for a brief period". Both implementations now hold such Objects while a SUBSCRIBE waits, up to 1 MiB, by one rule that both read from `fixtures/early-objects/`. Do the tests that read that table pass in both, and do the pairings of the viewers, the reader for agents, the gatherer and the transcriber pass as before?

## Expected

Written before the runs:

- `mise run check` passes. The tests of both implementations that read `arrival.tsv`, `settle.tsv` and `held-bytes.tsv` pass. So does the test of the Rust session in which two Objects read before their SUBSCRIBE_OK come after it, in order.
- `mise run interop -- --only watch` and `--only agent` pass three times each. `--only chat` and `--only transcriber` pass once. The whole run passes once.

## Environment

macOS 14.4, arm64, loopback, on `main` at `b8709f81` with this change. The relay, the gatherer and the clients were debug builds; the terminal viewer, the reader for agents and the transcriber were release builds. Bun 1.3.12, and Chromium 156.0.8078.4 of playwright-core.

## Result

- `mise run check`: OK. 21 tests of `packages/moqt/test/core/early-objects.test.ts`, 3 of `xmoq_moqt::early`, and 2 of the session.
- `--only watch`: `5 of 5 pairings ended as expected`, three times. `--only agent`: `4 of 4`, three times. `--only chat`: `11 of 11`. `--only transcriber`: `3 of 3`.
- The whole run (`mise run interop -- --base-port 33000`): `242 of 242 pairings ended as expected in 572.6s`.

**As expected.**

## What was not run

- The order of the failure in CI, on purpose: no run here made the SUBSCRIBE_OK come after its first Object. The pairing that failed passed in each run, as it did before the change most of the time.
- A peer that sends more than 1 MiB under an alias before its SUBSCRIBE_OK.
