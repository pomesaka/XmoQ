# What a refusal says of when to ask again

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the transport layer (REQUEST_ERROR)

## Objective

Every REQUEST_ERROR the two implementations made up themselves had a Retry Interval of 0, which reads as "do not retry". The relay got more ways to refuse on 2026-10-12 and 13 (a held SUBSCRIBE whose time is over, a PUBLISH_NAMESPACE that comes too often), after each of which the same request can succeed later. This is what the draft has on the value, what the peers do, and what was changed.

## What the draft says

All of it is in §10.6.2; no other section has a rule for it.

- The field: "Retry Interval: The minimum time (in milliseconds) before the request SHOULD be sent again, plus one. If the value is 0, the request SHOULD NOT be retried."
- When to set it: "If a request is retryable with the same parameters at a later time, the sender of REQUEST_ERROR includes a non-zero Retry Interval in the message. To minimize the risk of synchronized retry storms, the sender can apply randomization to each retry interval so that retries are spread out over time. A Retry Interval value of 1 indicates the request can be retried immediately."
- One code has a SHOULD for the sender: "EXCESSIVE_LOAD: The responder is overloaded and cannot process the request at this time. The sender SHOULD use the Retry Interval to indicate when the request can be retried." `UNAUTHORIZED` "might be retryable if the authorization token is not yet valid".
- One other use: "A relay MAY cache a REDIRECT response for a Full Track Name for up to Retry Interval milliseconds".
- **For the receiver there are a SHOULD and a SHOULD NOT, and no MUST**: nothing makes a request that is sent sooner, or after a 0, an error.
- **The draft does not say** what "the same parameters" covers (the Request ID is new in any case), what a value means with a code after which the same request cannot succeed, or an upper limit. The field is a `vi64`, which carries every 64-bit number (§1.4.1).

## What the peers do

Read in the sources, not seen on the wire with a value other than 0.

- moqtail, the relay at tag `relay@0.15.0` (the checkout cargo made for the interop peers): 0 in every REQUEST_ERROR it makes, with `//TODO: Maybe decide on another retry interval?` at six of the places. Its Rust and TypeScript libraries carry the field and do nothing with it; its test client prints it.
- moqtransport 0.14.0 (the version in the module cache that the build of moqlivemock fetched): the field is written and read by the wire code and named nowhere else, so it is 0 when sent and not handed to the application.

## What was changed

- Reading, in both libraries: `RequestFailedError.retryWaitMs` in `@xmoq/moqt`, `RequestError::retry_after_ms` and `retry_wait_ms` in the Rust session. The libraries hand the value on and do not wait themselves.
- A REQUEST_ERROR a session makes up: 1,001 (after a second) with `EXCESSIVE_LOAD`, 0 otherwise.
- The relay: 1 with `TIMEOUT`, 1,001 with `EXCESSIVE_LOAD` and `DOES_NOT_EXIST`, the time an announcement takes to come back for a PUBLISH_NAMESPACE that comes too often, 0 with the others.
- `@xmoq/media`, which asks again on the session for a viewer that waits: after the wait the refusal names, no sooner than 100 ms; after its own second where the refusal says 0 or is no REQUEST_ERROR.
- Shared tables for the two libraries: `fixtures/retry-interval/`.

## Commands

```sh
mise run check
mise run interop -- --base-port 48100
```

## Observed results

`mise run check` ended with 0, and `mise run interop` with "158 of 158 pairings ended as expected". Four of the runs look at the value, through the own relay:

| Run | Who reads it | What it had |
|---|---|---|
| `rendezvous: ... held SUBSCRIBE is refused with TIMEOUT` | own page | `RequestFailedError: REQUEST_ERROR 0x2:  (retry: after 0 ms)`: the library's `retryWaitMs` |
| the same | own Rust client | `SUBSCRIBE rejected: error_code=0x2 retry_interval=1` |
| `rendezvous: ... SUBSCRIBE without RENDEZVOUS_TIMEOUT is refused at once` | own Rust client | `SUBSCRIBE rejected: error_code=0x10 retry_interval=1001` |
| `quiet peer: the relay refuses a session's PUBLISH_NAMESPACE requests beyond five at once ...` | `xmoq-quiet-peer` | `ANNOUNCED sent=60 ok=7 refused=53 failed=0 asked=7 retry_interval=1001` |

No run has a client wait for the time it was told and ask again.

## Not run

- The watching page of the demo with `?wait=1` on this relay: its pause after a TIMEOUT becomes 100 ms where it was a second, and stays a second after DOES_NOT_EXIST.
- A peer that sends a Retry Interval other than 0: none of the peers does.
- The terminal viewer does not ask again after a refusal, so it has no use for the value.

## Next action

The numbers are provisional (`docs/open-questions.md`). Left as they were: `DUPLICATE_SUBSCRIPTION` with 0 where a moment later the request would be taken, and no randomizing.
