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
- `@xmoq/moqt`: `answerTimeoutMs` of the connect options. A request that is not answered is cancelled as `cancel()` does it and fails with `RequestAbortedError` of `no-answer-in-time`; a FETCH ends with `no-answer-in-time`.
- The Rust client: `--answer-wait-ms`. It prints `ANSWER_TIMED_OUT request=... after_ms=...`, cancels the request, and the mode fails.
- Nothing in the relay.

## Environment

One machine, macOS, loopback; Chromium 156.0.8078.4 of Playwright, headless. The own relay, and behind it the own Rust client as a publisher that announces the namespace and holds back its answer to the relay's SUBSCRIBE (`--serve-on-line`, never told to serve).

## Commands

```sh
cd interop
bun no-answer.ts --subscriber own-browser
bun no-answer.ts --subscriber own-rust
mise run interop        # both, among the others
```

The subscriber's wait is set to 1.5 seconds. WHY not the 10 it has by itself: the relay waits 5 seconds for a publisher and then answers the subscriber with `TIMEOUT`, so with 10 nothing stays unanswered.

## Observed results

| Subscriber | Gave up after | What it said | What the relay printed | The session |
|---|---|---|---|---|
| own-browser | 1,508 ms | `subscribe()` failed with `RequestAbortedError: the request ended without a response: no-answer-in-time` | `RESET_RECEIVED session=1 stream=1 code=0x1`; no answer to the SUBSCRIBE and no `CLOSE_SESSION` before it | open: `closed` had not settled, and a PUBLISH_NAMESPACE and a PUBLISH sent afterwards were answered |
| own-rust | 1,511 ms after it started | `ANSWER_TIMED_OUT request=subscribe after_ms=1500` | the same, and `STOP_SENDING_RECEIVED ... code=0x1`; then `CANCEL_OWN_REQUEST session=0 own=0 code=0x1`, its own SUBSCRIBE to the publisher | closed by the client as its mode failed, with NO_ERROR: `CONNECTION_ENDED session=1 ClosedByPeer { code: 0, reason: "" }` |

One run of each by hand, and both in the interop run.

## Not run

- **The 10 seconds themselves, against a real peer.** The tests of `@xmoq/moqt` move a clock by hand: 9,999 ms nothing, 10,000 ms given up, for SUBSCRIBE, PUBLISH, PUBLISH_NAMESPACE and FETCH, and 13,000 and 30,000 ms with the two parameters. The Rust client's time is tokio's, run with 1.5 seconds only.
- A FETCH, a PUBLISH and a PUBLISH_NAMESPACE that are not answered, against a real peer: the relay answers the last two itself at once, and no peer here holds a FETCH's answer back for longer than the relay waits.
- A relay that answers nothing at all. No such peer was built; the publisher behind the relay is what was silent.
- A page long in the background, whose timers run late.

## Next action

None from this record. The terminal viewer still gives SETUP and its first requests 10 seconds together and lets the connection go; the notes say so.
