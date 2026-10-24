# A silent subgroup stream is stopped

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: the receive limits of `@xmoq/moqt` (a gap in its bounds: a subgroup stream the peer opens and keeps open without bytes was held for as long as the session lasted)

## Objective

`@xmoq/moqt` now stops a subgroup stream of the peer that sends no byte for 30 seconds once its Subgroup Header is whole (`idleSubgroupStreamTimeoutMs`), with STOP_SENDING `INTERNAL_ERROR` (`0x0`). The time counts from the stream's last bytes and runs only while the stream is read, so a stream held back by a full subscription queue is not timed. What was held of the stream is dropped, it counts as ended for PUBLISH_DONE, and the subscription and the session stay. Fetch streams do not have this time. The draft has no such time (§11.4 and §11.4.3 are silent on it; §13.6 advises timeouts against "a peer that does not send expected data within an expected time"). The rule and its reasons are in the row "A subgroup stream of the peer that sends nothing after its header" of the [implementation notes](../moqt-implementation-notes.md). The Rust implementation gets the same rule in another change.

## Expected

**Written after the runs**, the unit tests and the interop run: the expectation was what the change was made to do.

- The tests of `@xmoq/moqt` pass with a fake clock (`bun:test` fake timers driving `setTimeout`): a stream that sent its header, or its header and an Object, and then nothing is stopped with code 0 at 30,000 ms and not at 29,999; bytes before then start the time again; the setting changes the time; 0, -1, 1.5, NaN, Infinity and 2^31 are a `RangeError`; another stream of the subscription and another subscription go on; Objects held for a Pending SUBSCRIBE are not delivered after the stop; a stream held back by a full queue for an hour is not stopped, and is stopped 30 seconds after it is read again; no timer is left after a FIN or after the stop.
- `bun interop/matrix.ts --only "own-browser" --base-port 45300` ends with every pairing as expected: no publisher in it leaves a subgroup stream silent for 30 seconds.

## Environment

macOS 14.4 (Darwin 23.4.0), arm64, loopback; Bun 1.3.14; the browser and peers of `interop/matrix.ts` (Chromium of playwright-core, headless); the working copy at change `xqysusnm`, with `cargo build --workspace` and `cargo build --locked --release -p xmoq-watch -p xmoq-quiet-peer -p xmoq-moqt-client -p xmoq-relay` run before the interop run.

## Result

- `mise run check` passed: the TypeScript checks (Biome, oxlint, type check, `bun test`; 1,311 tests of `@xmoq/moqt` among them, with `test/webtransport/idle-stream.test.ts` and `test/core/session-idle-stream.test.ts`), the layer checks, the workflow checks and the Rust checks.
- `bun interop/matrix.ts --only "own-browser" --base-port 45300`: 95 of 95 pairings ended as expected in 107.9 s.
- **Not run**: a peer that opens a subgroup stream towards a real Chromium page and then sends nothing. `xmoq-quiet-peer` has no such case; the stop is seen only against the fake transport of the tests, whose `reader.cancel` resolves a waiting read with its end as the Streams standard has it. How late a page in the background runs the 30 s timer was not measured; it is the same timer as the first-bytes one ([record](2026-10-11-first-bytes-timeout-in-chromium.md)), which runs late and never early.
- **Not run**: the Rust side, which is in another change.

After that, `mise run check` passed and `mise run interop -- --base-port 45300` ended 201 of 201 as expected; no pairing has a silent subgroup stream yet.
