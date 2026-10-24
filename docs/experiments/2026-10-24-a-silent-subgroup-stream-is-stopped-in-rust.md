# A silent subgroup stream is stopped, in Rust

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: the receive limits of `xmoq-moqt-io` (the same gap that [the record of `@xmoq/moqt`](2026-10-24-a-silent-subgroup-stream-is-stopped.md) closed there: a subgroup stream the peer opens and keeps open without bytes was held for as long as the session lasted)

## Objective

`xmoq-moqt-io` now stops a subgroup stream of the peer that sends no byte for 30 seconds once its Subgroup Header is whole (`StreamConfig::idle_subgroup_stream_timeout`), with STOP_SENDING `INTERNAL_ERROR` (`0x0`), as `@xmoq/moqt` does. The time is set anew before each read, after what the last read decoded was put in the session's queue, so the time a full queue held the stream back is not counted. The stream ends as `StreamEnd::Aborted(None)`; the subscription and the session stay. Fetch, control and padding streams do not have this time. The value is an `IdleSubgroupStreamTimeout` of 1 to 2^31 - 1 ms; the Rust client takes it as `--idle-subgroup-stream-ms`. The relay and the terminal viewer read their streams with `xmoq-moqt-io` and have the default. The rule and its reasons are in the row "A subgroup stream of the peer that sends nothing after its header" of the [implementation notes](../moqt-implementation-notes.md). This record checks that a peer which writes a Subgroup Header and nothing more is stopped by the own Rust client and by an own page in Chromium, over a real connection.

## Expected

Written before the interop runs, after the unit tests.

- The tests of `xmoq-moqt-io` pass on the in-memory connection with tokio's paused clock: a stream that sent its header and then nothing is closed as `Aborted(None)` and stopped with code 0 at 30,000 ms and not at 29,999; an Object at 20,000 ms moves that to 50,000; a time set to 500 ms with Objects at 300 and 700 stops it at 1,200; another stream is read afterwards; a stream held back by a full queue for an hour is not stopped and is stopped 30 seconds after it is read again; a control, a fetch and a padding stream are not stopped after an hour; `IdleSubgroupStreamTimeout::from_millis` refuses 0 and 2^31.
- In `bun interop/matrix.ts --only "quiet peer:" --base-port 45300`, the two new pairings (`--case silent-subgroup` with own-rust and with own-browser, each with an idle time of 1,000 ms) end as expected: `xmoq-quiet-peer --quiet silent-subgroup` prints `STOP_SENDING_RECEIVED session=0 code=0x0 after_ms=N` with N from 1,000 to 5,000 and no `SESSION_ENDED`; a second later own-rust has printed `STREAM_ABORTED uni#N` and has not left, and the page's state has no end of its session. The other pairings with `quiet peer:` end as before. Run three times.

## Environment

macOS 14.4 (Darwin 23.4.0), arm64, loopback; Bun 1.3.14; Rust as `rust-toolchain.toml` pins it; the browser of `interop/matrix.ts` (Chromium of playwright-core, headless); `cargo build --workspace` and `cargo build --locked --release -p xmoq-watch -p xmoq-quiet-peer -p xmoq-moqt-client -p xmoq-relay` run before the interop runs.

## Result

- The tests of `xmoq-moqt-io` pass, 116 in all, among them `a_subgroup_stream_that_sends_nothing_after_its_header_is_stopped_after_the_idle_time`, `the_time_a_full_queue_holds_a_subgroup_stream_back_is_not_idle`, `streams_that_are_not_subgroup_streams_have_no_idle_time` and `the_idle_time_of_a_subgroup_stream_is_from_1_ms_to_2_to_the_31_minus_1`; the Rust client's `the_idle_time_of_a_subgroup_stream_is_taken_from_1_ms_and_never_0` passes as well.
- **Two existing tests ran into the new time**, and were not about it: `at_most_the_configured_number_of_streams_is_read_at_once` of `xmoq-moqt-io` waits an hour with a stream open after its header, and `a_stream_is_given_up_after_the_smaller_of_the_flag_and_what_the_subscriber_asked` of the relay holds a stream of the relay undelivered for 60 seconds, which its test viewer then stopped after 30 (`Aborted(None)` where no end was expected). Both now give the reader the longest time, 2^31 - 1 ms.
- `mise run check` passed: the Rust checks (fmt, clippy, test), the TypeScript checks, the layer checks and the workflow checks.
- One run of the two new pairings alone (`--only "says nothing after its header"`) came before the Expected above was written, as a check of the driver; both ended as expected, stopped 1,004 ms after the header.
- `bun interop/matrix.ts --only "quiet peer:" --base-port 45300`, three times: 38 of 38 pairings ended as expected each time (64.3 s, 62.9 s, 62.4 s). The peer saw STOP_SENDING `0x0` after 1,003, 1,002 and 1,004 ms from own-rust, and after 1,005, 1,003 and 1,002 ms from own-browser; own-rust printed `STREAM_ABORTED uni#1` and stayed, and the page's state was `{"session":null,"objectsRead":0,"objectsEnd":null}` a second later.
- **Not run**: the relay against a publisher that leaves a subgroup stream silent, and the terminal viewer against such a peer. Both read their streams with the same code of `xmoq-moqt-io` and have the default of 30 seconds and no flag to shorten it, so a run would take more than 30 seconds a pairing.
- **Not run**: a stream held back by a full queue over a real connection; that part is seen only on the in-memory connection.

After that, the change was put on `main` with `--quiet datagram-before-setup` (#302) beside it; the four files both changed were merged by keeping both. `mise run check` passed, and `bun interop/matrix.ts --only "quiet peer:" --base-port 45300` ended 41 of 41 as expected three times running. Then `mise run interop -- --base-port 45300` ended 207 of 207 as expected (441 s).
