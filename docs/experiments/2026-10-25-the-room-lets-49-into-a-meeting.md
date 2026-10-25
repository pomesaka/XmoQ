# The room lets 49 into a meeting

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, rooms; meetings.

## Objective

The 49 participants of a meeting are counted by the room, which gives no tokens beyond them (decided on 2026-10-25). The room answers `join-meeting` with a token to publish in `xmoq/meet/<room>/<id>` and one to subscribe to `xmoq/meet/<room>/*`, or `meeting-full`. Does it count as `spikes/durable-objects-room/README.md` says, with a key and without?

## Expected

Written before the runs: `check.mjs` against `run.mjs`, without `AUTH_KEY_HEX` and with one, prints `ALL CHECKS PASSED` both times. Its part 5:

- 49 sockets that join with `p0` to `p48` each get `meeting` with their id, `publish` `["xmoq","meet",<room>,"p0"]` and `subscribe` `["xmoq","meet",<room>,"*"]`; the tokens null without a key, and `v1|xmoq/meet/<room>/p0|pub|...` and `v1|xmoq/meet/<room>/*|sub|...` with it.
- The 50th gets `{"type":"meeting-full","most":49}`.
- No answer to an empty id, one of 65 characters, `a/b`, `*`, a number, another id on a socket that joined, an id another open socket has (asked when there is room), or join-meeting in a room named `a/b`.
- One of the 49 asking again with its id while the meeting is full gets `meeting` again.
- When one of the 49 closes, the 50th gets in with `p49`.
- `token-for` `pub` in `xmoq/meet/<room>/x` is not answered.

## Environment

macOS 14.4, arm64, loopback; Node 23.10.0 and Miniflare of `spikes/durable-objects-room`, on `main` at `9c91aec1` with this change. The rooms on port 33987, with no other variable than `AUTH_KEY_HEX` in the second run.

## Result

**As expected**, both runs `ALL CHECKS PASSED`. With the key:

```
the first one's answer: {"type":"meeting","participant":"p0","publish":{"namespace":["xmoq","meet","check-1792907483312-meeting","p0"],"token":"v1|xmoq/meet/check-1792907483312-meeting/p0|pub|1792908087|<mac>"},"subscribe":{"namespace":["xmoq","meet","check-1792907483312-meeting","*"],"token":"v1|xmoq/meet/check-1792907483312-meeting/*|sub|1792908087|<mac>"}}
the 50th one's answer: [{"type":"meeting-full","most":49}]
the 50th one's answer after one went: "meeting"
```

## What was not run

- The tokens at a relay: no participant published or subscribed with them.
- A 49th and a 50th join-meeting that come while the room makes a token: the room counts a socket before it makes the tokens, which no run tells apart from counting after.
- These in `mise run interop`, which runs no room.
