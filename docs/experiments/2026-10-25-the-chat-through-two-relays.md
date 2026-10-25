# The chat through two relays

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale; the third step of building the own relay into an edge of the [design draft](../scale-and-meetings.md) (PUBLISH_NAMESPACE passed upstream, §9.5)

## Objective

The chat's gatherer watches the namespaces under `xmoq/chat` at its relay with SUBSCRIBE_NAMESPACE, and a participant announces `xmoq/chat/<room>/<name>` at its own. With the gatherer at one relay and the participants at a relay below it, the gatherer hears them only if the relay below passes their announcements up. Does it, once for each namespace, and does it withdraw one when its participant leaves?

## Expected

Written at the top of `interop/chat-edge.ts` before the first run:

- The gatherer is told of `a` and `b`, both at the relay below, and subscribes to each one's `said`; each participant serves the subscription.
- The relay below announces each namespace upstream once (two PUBLISH_NAMESPACE in all).
- Lines of `a` and `b`, one after the other, are numbered 0 to 3, and a viewer at the relay below gets them live: Groups 0 to 3, whose JSON names a, b, a, b.
- A viewer at the relay below that comes then FETCHes 0 to 100 and gets Groups 0 to 3, with `FETCH_OK` ending at {3, 1}.
- When `a` leaves, the relay below withdraws its namespace upstream, and the relay above tells the gatherer NAMESPACE_DONE.

## Environment

macOS 14.4, arm64, loopback; both relays the own one (`xmoq-relay/0.0.0`), the one below on the run's port plus 1,000 with `--upstream`; the gatherer `xmoq-chat` and own-rust clients; `mise run interop -- --base-port 34500 --only "edge: chat"`.

## Result

**As expected**, in the run after the driver was given the `--relay` flag the matrix passes (the first run ended at once on the unknown flag), 10.5 s. The live viewer got `0:a 1:b 2:a 3:b`; the late one fetched Groups 0 to 3, `FETCH_OK` {3, 1}.

The two relays:

```
[relay:second] CONNECT_UPSTREAM uri= after_ms=0
[relay:second] UPSTREAM_CONNECTED session=1
[relay:second] OPEN_REQUEST session=1 own=0 PUBLISH_NAMESPACE
[relay:second] OPEN_REQUEST session=1 own=2 PUBLISH_NAMESPACE
...
[relay:second] CONNECTION_ENDED session=0 Lost("connection error: timed out")
[relay:second] CANCEL_OWN_REQUEST session=1 own=0 code=0x1
[relay] SEND session=0 stream=1 NAMESPACE_DONE suffix=r/a
```

Most of the 10.5 s is the wait for `a`, killed with SIGTERM, to be found lost by the relay below (its idle timeout); the withdrawal followed at once.

## Not looked at

- Pages as the participants (`@xmoq/chat`), and `--auth`: the relay below carries no token upstream, and a SUBSCRIBE of the relay above carries none to it.
- A participant that withdraws its namespace and stays connected; it is in the relay's tests (`a_namespace_two_announce_is_withdrawn_upstream_when_the_last_of_them_withdraws`), not in this run.
- The announcements sent again on a new session with the upstream, and the SUBSCRIBE of the upstream that would go back up: in the relay's tests alone.
- At the end of the run the relay above was stopped with a GOAWAY without a URI, and the relay below, which still announced `b`, connected to it again at once, as §10.4 has for a GOAWAY without one; nothing was asked of that session before the run ended.
