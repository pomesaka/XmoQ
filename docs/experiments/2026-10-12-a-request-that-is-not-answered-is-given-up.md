# A request that is not answered is given up

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the transport layer in two implementations (limits on what a peer decides)

## Objective

`@xmoq/moqt` and the Rust client waited for the answer to a SUBSCRIBE, a FETCH, a PUBLISH and a PUBLISH_NAMESPACE without end: against a relay that does not answer, a page stayed "connecting". Whether both can give such a request up after the same time, in the same way, and keep their session.

## What the draft says

Nothing of how long. §13.6: "Each implementation is expected to set its own timeouts." §3.5 has a code for closing the session: "CONTROL_MESSAGE_TIMEOUT (0x11): The session was closed because the peer took too long to respond to a control message." The request alone is given up here, and the session kept; the row "A request the peer does not answer" of [the implementation notes](../moqt-implementation-notes.md) has the rule and why.

## What was built

- The time, in both: 10 seconds, and as long again as the RENDEZVOUS_TIMEOUT of a SUBSCRIBE or the FILL_TIMEOUT of a FETCH (`answer_wait_ms` in `xmoq-moqt`, `answerWaitMs` in `@xmoq/moqt`). Ten rows are shared ([`fixtures/request-answer-wait/`](../../fixtures/request-answer-wait/README.md)); the tests of both read them.
- `@xmoq/moqt`: `answerTimeoutMs` of the connect options. A request that is not answered is cancelled as `cancel()` does it and fails with `RequestAbortedError` of `no-answer-in-time`; a FETCH ends with `no-answer-in-time`. A PUBLISH is ended with PUBLISH_DONE (`INTERNAL_ERROR`, Stream Count 0) and a FIN. The time begins when the request is written, and an answer the session has read is not timed out while it waits to be handed on.
- The Rust client: `--answer-wait-ms`. It prints `ANSWER_TIMED_OUT request=... after_ms=...`, gives the request up in the same way, and the mode fails.
- The terminal viewer: the same time for each SUBSCRIBE (`--answer-wait-ms`), 10 seconds for the SETUP. **Found on the way**: when the viewer failed before it played, it let go of its control stream and of the relay's before it closed the session, and the relay closed first, with PROTOCOL_VIOLATION (`the peer closed its control stream`, then `the peer stopped the relay's control stream`). Both are now kept until the close.
- Nothing in the relay.

## Environment

One machine, macOS, loopback; Chromium 156.0.8078.4 of Playwright, headless. The own relay, and behind it the own Rust client as a publisher that announces the namespace and holds back its answer to the relay's SUBSCRIBE (`--serve-on-line`, never told to serve).

## Commands

```sh
cd interop
bun no-answer.ts --subscriber own-browser
bun no-answer.ts --subscriber own-rust
bun no-answer.ts --subscriber xmoq-watch
mise run interop        # both, among the others
```

The subscriber's wait is set to 1.5 seconds. WHY not the 10 it has by itself: the relay waits 5 seconds for a publisher and then answers the subscriber with `TIMEOUT`, so with 10 nothing stays unanswered.

## Observed results

| Subscriber | Gave up after | What it said | What the relay printed | The session |
|---|---|---|---|---|
| own-browser | 1,508 ms | `subscribe()` failed with `RequestAbortedError: the request ended without a response: no-answer-in-time` | `STOP_SENDING_RECEIVED session=1 stream=1 code=0x1` and `RESET_RECEIVED session=1 stream=1 code=0x1`: **Chromium sends both, with the code**; no answer to the SUBSCRIBE and no `CLOSE_SESSION` before them | open: `closed` had not settled, and a PUBLISH_NAMESPACE and a PUBLISH sent afterwards were answered |
| own-rust | 1,511 ms after it started | `ANSWER_TIMED_OUT request=subscribe after_ms=1500` | the same, and `STOP_SENDING_RECEIVED ... code=0x1`; then `CANCEL_OWN_REQUEST session=0 own=0 code=0x1`, its own SUBSCRIBE to the publisher | closed by the client as its mode failed, with NO_ERROR: `CONNECTION_ENDED session=1 ClosedByPeer { code: 0, reason: "" }` |

| xmoq-watch | 2,248 ms after it started | `Error: the relay did not answer the SUBSCRIBE of object within 1500 ms` | the same two lines | closed by the viewer with NO_ERROR: `CONNECTION_ENDED session=1 ClosedByPeer { code: 0, reason: "" }` |

One run of each by hand, and all three in the interop run.

## Against a peer that stays quiet

Added later the same day. `xmoq-quiet-peer` (`crates/quiet-peer`) is a server built for this: it takes a session and sends no SETUP, or sends its SETUP and answers no request (one mode answers a PUBLISH_NAMESPACE, so that a client comes to its PUBLISH), and prints what reached it. No relay is in between. `bun quiet-peer.ts --case ... --client ...`, the clients' waits at 1.5 seconds; one run of each by hand, and all seven in the interop run.

| Case | Client | What the peer printed |
|---|---|---|
| No SETUP | own-browser, own-rust, xmoq-watch | `SESSION_ENDED session=0 ClosedByPeer { code: 17, reason: "" }` (0x11, CONTROL_MESSAGE_TIMEOUT), 1,755, 2,302 and 2,220 ms after the client began |
| No answer to a PUBLISH | own-browser, own-rust | `REQUEST session=0 stream=1 PUBLISH_DONE status_code=0x0 stream_count=0 reason="no answer in time"`, then `REQUEST_CLOSED session=0 stream=1 how=FIN` |
| No answer to a FETCH | own-browser, own-rust | `REQUEST session=0 stream=0 FETCH`, then `REQUEST_CLOSED session=0 stream=0 how=reset, code 0x1`; own-rust printed `ANSWER_TIMED_OUT request=fetch#0 after_ms=1500` and left with a failure |

The peer does not watch whether its own direction of a request stream is stopped, so the STOP_SENDING of these is not seen here; for a SUBSCRIBE the relay's log above has it.

**The relay's side was read, not run**: no timer for a client's SETUP was found in `crates/relay-core` or `crates/relay/src/hub.rs`, so a client that connects and sends none appears to stay for as long as its connection lives. No client that sends no SETUP is at hand to run it.

## Not run

- **The 10 seconds themselves, against a real peer.** The tests of `@xmoq/moqt` move a clock by hand: 9,999 ms nothing, 10,000 ms given up, for SUBSCRIBE, PUBLISH, PUBLISH_NAMESPACE and FETCH, and 13,000 and 30,000 ms with the two parameters. The Rust client's time is tokio's, run with 1.5 seconds only.
- A PUBLISH_NAMESPACE that is not answered, against a real peer. A publisher that has Objects and holds its FETCH_OK back, behind the relay: the FETCH that was run is against the quiet peer, which answers nothing.
- A relay that answers nothing at all. No such peer was built; the publisher behind the relay is what was silent.
- A page long in the background, whose timers run late.

## Next action

None from this record.
