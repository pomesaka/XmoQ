# Two relays that ask for tokens

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale; the fourth step of building the own relay into an edge of the [design draft](../scale-and-meetings.md) (its own tokens upstream)

## Objective

With `--auth-key-hex` a relay closes a session whose SETUP has no token of the room and refuses a request without one (`crates/relay/src/auth.rs`). A relay below another has to be let in by the one above, and its viewers still checked. Does a relay below, given the same key, make tokens of its own that the relay above takes for its SETUP and its SUBSCRIBE, while it takes what comes from its upstream without a token (decided on 2026-10-25) and checks its viewers as before?

## Expected

Written at the top of `interop/auth-edge.ts` before the first run:

- The publishing page connects to the relay above with `pub` for `xmoq/auth/*` and publishes `xmoq/auth`/`object`.
- A page at the relay below with `sub` for `xmoq/auth/*` subscribes: the relay below connects upstream and is let in with its own token, its SUBSCRIBE is accepted, and the page gets the Object the publisher sends.
- A page at the relay below with `sub` for `xmoq/other/*` is refused its SUBSCRIBE by the relay below with REQUEST_ERROR 0x1 (UNAUTHORIZED).
- `chat-tokens.ts --through-edge`: what `chat-tokens.ts` has to show through one relay (its head, written before its first run), with the pages at the relay below and the gatherer above, all with the same key. The relay below passes the pages' PUBLISH_NAMESPACE up with a `pub` token of its own, and the gatherer's SUBSCRIBEs come down to the pages through it without a token.

## Environment

macOS 14.4, arm64, loopback; Chromium of Playwright; both relays the own one (`xmoq-relay/0.0.0`) with the same `--auth-key-hex` and the SETUP checked (`--auth-setup` on, the default), the one below on the run's port plus 1,000 with `--upstream`; `mise run interop -- --base-port 34500 --only "edge: auth"`, and with the PUBLISH_NAMESPACE passed up (after #334) `--only "edge:"`.

## Result

**As expected**, in the first run, 1.7 s. The viewer with a token for another namespace got `REQUEST_ERROR 0x1` from the relay below.

The two relays:

```
[relay:second] CONNECT_UPSTREAM uri=
[relay] SESSION_OPENED session=1
[relay:second] UPSTREAM_CONNECTED session=1
[relay:second] OPEN_REQUEST session=1 own=0 SUBSCRIBE
[relay] SEND session=1 stream=1 SUBSCRIBE_OK track_alias=0
[relay:second] SEND session=0 stream=1 SUBSCRIBE_OK track_alias=0
[relay:second] SEND session=2 stream=1 REQUEST_ERROR error_code=0x1
```

The relay above kept the session of the relay below (no close for authorization) and accepted its SUBSCRIBE; the relay below kept its session with the relay above, whose SETUP carries no token.

**The chat with tokens through the two relays, as expected**, in its first run, 3.0 s: pages a and e had both lines live, f had them as history by its Joining FETCH, b and c were refused their PUBLISH_NAMESPACE and d its SUBSCRIBE with 0x1. The relay below passed up four PUBLISH_NAMESPACE (a, e, f, d), the three refusals were its own (`[relay:second] SEND ... REQUEST_ERROR error_code=0x1`), and the relay above printed no refusal for authorization.

## Not looked at

- FETCH and TRACK_STATUS of the relay below with its token: in the relay-core tests alone.
- A token that expires while a subscription of the relay below lasts (the FIXME of `auth.rs` holds for every subscription).
