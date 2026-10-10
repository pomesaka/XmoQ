# `@xmoq/moqt` waits for the subscriber after PUBLISH_DONE

- Date: 2026-10-10
- Commit SHA: `a440f0cf552341283ec3aafcf787f9e94fb0fb2e`. The runs were made on its working copy on top of `4fc65f5`
- Roadmap item: Phase 0b, own TypeScript browser client. Closes the gap the [implementation notes](../moqt-implementation-notes.md) name in the row "A publisher that ended its PUBLISH with PUBLISH_DONE and wants to close the session": the Rust client did this since [the change of library](2026-10-10-webtransport-library-switch.md), the browser client did not

## Objective

Make the browser client do what the Rust client does after its PUBLISH_DONE: wait for the subscriber to end its direction of the request stream, report how it did, and only then let the application close the session. Show in a real Chromium that a page built on the library closes after the relay's FIN.

### What the two implementations have to agree on

| # | Requirement | From |
|---|---|---|
| 1 | After PUBLISH_DONE and FIN, wait up to 3 seconds for the subscriber to end its direction | The row of the notes; `PUBLISH_DONE_ANSWER_WAIT` in `crates/moqt-client/src/main.rs` |
| 2 | Tell a FIN, a reset with its code, and nothing in time apart, and treat none as an error | The row; `report.rs` of the Rust client |
| 3 | Name what was seen and no more: a FIN means "everything arrived" only from an XmoQ relay | The row |
| 4 | Do not wait when the subscriber had finished its direction before PUBLISH_DONE | `finish_and_await_close` in `crates/moqt-client/src/io.rs` |
| 5 | Ignore what the subscriber still sends on the stream; a REQUEST_UPDATE still uses up its Request ID | §5.1, §10.1; unchanged. Only `@xmoq/moqt` records that Request ID; the Rust session does not check it (a known gap of the notes) |
| 9 | The same wait after the PUBLISH_DONE `UPDATE_FAILED` that answers a refused REQUEST_UPDATE | `publish` in `crates/moqt-client/src/main.rs` goes through the same `finish_and_await_close` |
| 6 | When the wait is over, stop the subscriber's direction with STOP_SENDING (CANCELLED) | `finish_and_await_close` |
| 7 | A STOP_SENDING from the subscriber does not end the wait: this endpoint has finished its direction | `finish_and_await_close` reads the receiving half only |
| 8 | The wait ends with the session, and leaves no timer | The rule on clean-up |

Only a PUBLISH is covered. The Rust client waits in the same way after it served a peer's SUBSCRIBE; the browser client does not serve a SUBSCRIBE.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4, arm64, loopback only |
| Browser | Chromium 156.0.8078.4, new headless, launched by `playwright-core` 1.64.0 |
| Relay and own-rust | `xmoq-relay` and `xmoq-moqt-client` 0.0.0 at `4fc65f5`, `web-transport-quinn` 0.13.2, debug builds |
| moqtail relay | 0.15.0, as prepared by `interop/README.md` |
| Toolchain | `bun` 1.3.12, TypeScript 6.0.3 |
| MoQ Transport | draft 18 |

## Commands

From the repository root:

```sh
mise run check
mise run interop
cd interop && bun matrix.ts --relay own --only "leaving: " --base-port 47000   # 20 times in a row
```

Three checks were made with a temporary change that was taken back afterwards and is not in the tree:

- In `packages/moqt/src/core/requests.ts`, the report of the subscriber's end was switched off; then `bun test` in `packages/moqt`.
- In `interop/page.ts`, the page connected with `peerStreamEndTimeoutMs: 0`; then `bun matrix.ts --relay own --only "leaving: own-browser publisher closes" --base-port 47000`.
- `packages/moqt/src/webtransport/connection.ts` was put back to its state before the review, for the test of two calls of `done()`, and the wait after UPDATE_FAILED was taken out, for its test; then `bun test`.

## Observed results

### The change

`PublishedTrack.done()` resolved when PUBLISH_DONE and the FIN were sent. It now resolves when the subscriber has ended its direction or the wait is over, with a `PeerStreamEnd`: `fin`, `reset` with its code, `nothing-in-time`, or `session-closed`. `PublishedTrack.ended` still resolves with `done` when PUBLISH_DONE was sent. The wait is `peerStreamEndTimeoutMs` of the connect options, 3000 by default; a value that is not a whole number from 0 to 2^31 - 1 is refused, as a timer fires at once for anything else, and the two other waits of the options (`publishDoneTimeoutMs`, `setupTimeoutMs`), which were not checked at all, go through the same check. With 0, `done()` does not wait and stops the subscriber's direction at once. The wait is counted from the moment the transport has taken PUBLISH_DONE and the FIN; the Rust client counts from its FIN.

A second `done()` rejects with `PublishEndedError`, at whatever moment it comes. In the first version of this change, a second call made before the first had written PUBLISH_DONE took the wait of the first away, which then never settled; a review found it, and the test for it ran into its time limit on that version. A second call of that kind was also refused with `SubgroupClosedError` before this change.

After the PUBLISH_DONE `UPDATE_FAILED` that the session sends itself, the same wait runs and the subscriber's direction is stopped at its end (requirement 9). Nobody is told how it ended.

WHY in `done()` and not elsewhere: an application that writes `await track.done()` and then `connection.close()` is right without knowing of the convention. A separate promise for the subscriber's end would leave that same code losing the last Object, which is what the interop page did. A `close()` that waits would delay a close for a protocol error by up to 3 seconds.

The session state (`src/core/requests.ts`) reports the end of the subscriber's direction after PUBLISH_DONE as an event and has an input for the end of the wait; the timer is in `src/webtransport/connection.ts`. That the subscriber of an Established PUBLISH finished its direction is kept in one place, where that FIN is handled, and read when PUBLISH_DONE is sent.

### Tests against the played transport

| Case | `done()` resolves with |
|---|---|
| The subscriber finishes its direction | `fin` |
| The subscriber resets it with code 1 | `reset`, code 1 |
| The subscriber sends a REQUEST_UPDATE and then finishes | `fin` |
| The subscriber had finished before PUBLISH_DONE, with a wait of an hour set | `fin`, at once |
| Nothing, with a wait of 0 | `nothing-in-time`; the subscriber's writer fails with stream error code 1 |
| The subscriber sends STOP_SENDING, with a wait of an hour set; then it finishes | not settled after the STOP_SENDING; `fin` after the FIN |
| The application closes the session while `done()` waits | `session-closed` |
| The transport fails while `done()` waits | `session-closed`, closed for an error |
| The transport has failed when `done()` is called | rejects; the session ends for the error |
| `done()` twice in a row, then the subscriber finishes | the second rejects with `PublishEndedError`, the first resolves with `fin`; a third call rejects in the same way |
| A REQUEST_UPDATE is refused, with a wait of 0 | `ended` is `update-failed`; the subscriber's writer fails with stream error code 1 |

No test waits for a clock: a wait is 0 or an hour.

`mise run check` passes, with 392 TypeScript tests. With the report of the subscriber's end switched off in the session state, 11 of the 380 tests of the first version failed. On `connection.ts` as it was before the review, the test of two calls ran into its time limit, and without the wait after UPDATE_FAILED so did its test.


### In Chromium

The interop page keeps what `done()` resolved with and prints it as `PUBLISH_DONE_PEER_ENDED how=…`, the line of the Rust client. The driver of the leaving scenarios fails when an own publisher, browser or Rust, saw the own relay end its direction in another way than with a FIN. That check is new for the Rust client as well: its line was printed and not looked at.

| Run | Result |
|---|---|
| `mise run interop` | 46 of 46 pairings |
| The 14 leaving scenarios, 20 rounds, on the first version of this change (before the review; `interop/` did not change after) | 20 of 20 rounds. `how={"type":"fin"}` from the page in 80 of 80 PUBLISH_DONE, `how=FIN` from the Rust client in 340 of 340 |
| The same 20 rounds on the change as committed | 20 of 20 rounds. `fin` from the page in 80 of 80, `FIN` from the Rust client in 340 of 340 |
| The page connected with `peerStreamEndTimeoutMs: 0`, the scenario "own-browser publisher closes right after its last Object and PUBLISH_DONE" | fails: `saw the relay end its direction of the request stream as {"type":"nothing-in-time"}, not with a FIN` |

What a client saw after its PUBLISH_DONE in one `mise run interop`, by relay. Through the moqtail relay it is recorded and not judged: what that relay means by finishing the stream was not looked at.

| Relay | own-browser | own-rust |
|---|---|---|
| own | `fin` in 6 of 6 | `FIN` in 20 of 20 |
| moqtail 0.15.0 | `fin` in 2 of 2 | `FIN` in 3 of 3 |

## Limitations and blockers

- **That the wait keeps an Object from being lost was not shown.** The loss needs a stream that arrived and was not accepted when the session closed, and no run had produced it when this was written: the scenario in which the page closed right after its last Object did not fail for a lost Object in any round of [the disconnect record](2026-10-10-disconnects-through-own-relay.md). **On 2026-10-11 it did, without PUBLISH_DONE: once in CI and in 1 of 200 runs on a loaded machine** (that record, "The last Object of a publisher that sends no PUBLISH_DONE can be lost"). With PUBLISH_DONE and the wait, no run has lost an Object; that is still no proof that the wait prevents it. What is shown is the order: the page closes after the relay's FIN.
- `reset` and `nothing-in-time` were seen with the played transport only, and so was the code of the STOP_SENDING at the end of a wait: in Chromium it was not observed.
- When `writer.close()` of a stream resolves in Chromium, at the FIN going out or at its arrival, was not looked at; the start of the wait depends on it. No run made the relay give up on the counted streams.
- Chromium only.
- A page that calls `close()` without waiting for `done()` is not kept from it; the doc of `done()` says what can happen.

## Open questions raised

- The 3 seconds are the Rust client's value and provisional like it. Against a peer that never ends its direction, every `done()` takes that long.
- Should `close()` warn, through the diagnostics hooks, when a `done()` is still waiting?
