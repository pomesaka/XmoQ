# A session the relay closes with a code, as the peer sees it

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, authorization (and every close of a session by the relay)

## Objective

A page that the relay turned down at SETUP (`--auth-setup on`, no token) got one of four errors, by the run: the close with its code, its control stream stopped, the peer's control stream closed, or a stream aborted. Whether the relay can close a session so that the peer learns of the close first.

## What the relay did

On `CLOSE_SESSION` the hub closed the connection, which has the library write the CLOSE_WEBTRANSPORT_SESSION capsule in a task of its own, and forgot the session in the same turn. Forgetting it stops the tasks that hold its streams: a sending half that is dropped is reset, a receiving half is stopped, and the subgroup streams towards the peer were reset with `SESSION_CLOSED`. Those frames went out at once, beside the capsule.

Now the hub keeps the session's streams as they are in a task until the connection is gone, 2 seconds at most, and lets go of them then. The draft says nothing of the streams of a session that is terminated (§3.5).

## Environment and commands

The own relay, built before and after the change, with `--auth-key-hex 00112233 --auth-setup on`; the Rust client of the working copy, `--mode subscribe` without a token, 10 times against each. Loopback, macOS. The client prints `SESSION_CLOSED_BY_PEER code=... reason=...`, new, when it failed and the connection says within 500 ms that the peer closed the session.

## Observed results

What the client failed with first, and whether it read the code:

| Relay | The first failure of the client | Runs | `SESSION_CLOSED_BY_PEER code=0x2 reason="authorization"` |
|---|---|---|---|
| before | "connection lost" | 6 | 10 of 10 |
| before | "protocol violation: the peer closed its control stream" | 3 | |
| before | "the peer closed the subscribe stream without a response: FIN" | 1 | |
| after | "connection lost" (once with the code in it) | 10 | 10 of 10 |

Before the change a stream ended first in 4 of 10 runs; after it, in none.

## Limitations and blockers

- **No browser.** What a page gets is for a run with one.
- The Rust client read the code in every run before the change as well: it asks the connection, which a page may not get to once a stream has failed.
- One case, the refusal at SETUP. The relay closes sessions for protocol errors by the same path; none of those was run.

## Decision or next action

The change stays. A page is to be tried against it.
