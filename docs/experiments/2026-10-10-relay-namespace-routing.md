# Routing by namespace in the own relay

- Date: 2026-10-10
- Commit SHA: the commit that adds namespace routing together with this record. The runs were made on its working copy
- Roadmap item: Phase 0b, route by namespace in the relay

## Objective

A subscriber asks the own relay for a track that no publisher has sent PUBLISH for. A publisher has announced the track's namespace with PUBLISH_NAMESPACE. The relay has to send its own SUBSCRIBE to that publisher, wait for the answer, and then serve the subscriber (draft 18, §9.4 and §9.5). The Object has to arrive with its payload unchanged, at subscribers written by someone else.

A publisher that announces its namespace and then sends PUBLISH can find the relay's SUBSCRIBE for the same track on its way to it. The relay's SUBSCRIBE has to give way to the PUBLISH (§5.1), and the subscribers that waited have to be served from it.

The same change makes a requester's FIN no cancellation (§5.1, §3.3.2), in the relay and in both clients.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Own relay | `xmoq-relay` 0.0.0 on `web-transport-quinn` 0.13.2 |
| Publisher, runs 1 to 5 | `xmoq-moqt-client` 0.0.0 in the new mode `--mode publish-namespace`: it sends PUBLISH_NAMESPACE and no PUBLISH, answers the SUBSCRIBE it then gets with SUBSCRIBE_OK, and sends one Object per line on stdin |
| Publisher, runs 6 to 9 | `xmoq-moqt-client` in `--mode publish` with `--publish-delay-ms 3000 --answer-delay-ms 4000`: it sends PUBLISH_NAMESPACE, waits 3 seconds, sends PUBLISH, and holds its answer to a request of the relay for 4 seconds |
| Publisher, run 10 | moqtail-ts in Chromium: `publishNamespace` and `addOrUpdateTrack`, and no `publish` |
| Subscribers | the moqtail Rust client (`moqtail` crate 0.15.0, `wtransport` 0.7.2); moqtail-ts (`moqtail` npm 0.13.0) in Chromium 156.0.8078.4, headless, through `playwright` 1.64.0; `xmoq-moqt-client` |
| Specification | `draft-ietf-moq-transport-18` |

Provisional settings, none of them project decisions: a publisher gets 5 seconds to answer the relay's SUBSCRIBE; 64 subscribers may wait for one answer; the relay has at most 64 requests open on one publisher; a session may announce 64 namespaces; 128 bytes of a publisher's Error Reason are passed on. No authorization.

## Commands

```sh
mise run check
mise run interop
cargo build --workspace

R="node spikes/moqtail/browser/run.mjs --relay own --rust-publisher own --publish-namespace"
$R --direction rust-to-rust --rust-subscriber own --await-publish-done      # 1
$R --direction rust-to-rust --rust-subscriber moqtail                       # 2
$R --direction rust-to-browser                                              # 3
$R --direction rust-to-browser --subscribers 3                              # 4
$R --direction rust-to-rust --rust-subscriber own --subscribers 2 --await-publish-done   # 5

S="node spikes/moqtail/browser/run.mjs --relay own --rust-publisher own --subscribe-before-publish"
$S --direction rust-to-rust --rust-subscriber own                           # 6
$S --direction rust-to-rust --rust-subscriber moqtail                       # 7
$S --direction rust-to-browser                                              # 8
$S --direction rust-to-browser --subscribers 3                              # 9

node spikes/moqtail/browser/run.mjs --relay own --direction browser-to-rust \
  --rust-subscriber own --publish-namespace --trace                         # 10
```

A different `--port` was passed per run.

The delays in runs 6 to 9: the order in question needs the relay's SUBSCRIBE to be unanswered when the PUBLISH arrives. On a real link that is the round trip between the relay accepting the namespace and the publisher's PUBLISH. On loopback the window is too short to hit on purpose, and every publisher at hand answers a SUBSCRIBE at once. The two flags of the own client widen the window; nothing else about the exchange is changed.

## Observed results

**Runs 1 to 9 passed: every subscriber received the payload that was sent. Run 10 failed: the own relay closed the session of the moqtail-ts publisher.**

Runs 1 to 5, routing to a publisher that only announced its namespace:

| # | Subscriber | Result | What the relay logged about its own request |
|---|---|---|---|
| 1 | own Rust client | payload matched; the subscriber saw PUBLISH_DONE | `OPEN_REQUEST ... SUBSCRIBE`, then `FINISH_OWN_REQUEST` after the publisher's PUBLISH_DONE and the one counted stream |
| 2 | moqtail Rust client | payload matched | `OPEN_REQUEST ... SUBSCRIBE`, then `CANCEL_OWN_REQUEST ... code=0x1` when the subscriber left |
| 3 | moqtail-ts in Chromium | payload matched | the same as 2 |
| 4 | three moqtail-ts pages | payload matched at all three | one `OPEN_REQUEST` for the three subscribers; cancelled when the last one left |
| 5 | two own Rust clients | payload matched at both; both saw PUBLISH_DONE | one `OPEN_REQUEST`, then `FINISH_OWN_REQUEST` |

In every run the publisher printed `SERVING track_alias=0` after the relay's SUBSCRIBE reached it, and in runs 1 and 5 `PUBLISH_DONE_PEER_ENDED how=FIN`: the relay finished its direction of its SUBSCRIBE once the stream PUBLISH_DONE counted had ended.

Runs 6 to 9, a PUBLISH that crosses the relay's SUBSCRIBE:

| # | Subscriber | Result |
|---|---|---|
| 6 | own Rust client | payload matched |
| 7 | moqtail Rust client | payload matched |
| 8 | moqtail-ts in Chromium | payload matched |
| 9 | three moqtail-ts pages | payload matched at all three |

In each of them the relay logged, in this order: `OPEN_REQUEST ... SUBSCRIBE` to the publisher when the first subscriber asked, `CANCEL_OWN_REQUEST ... code=0x1` when the PUBLISH arrived, the `REQUEST_OK` for the PUBLISH, and `SUBSCRIBE_OK` for every subscriber that had waited. The script waits for these lines, so a run in which the SUBSCRIBE was answered first would not have counted.

Run 10, moqtail-ts as the publisher that answers the relay's SUBSCRIBE: **failed, by a deviation of moqtail-ts.**

```
[relay] OPEN_REQUEST session=0 own=0 SUBSCRIBE
[relay] CLOSE_SESSION session=0 code=0x3 reason="protocol violation: parameter not allowed in this message"
[relay] SEND session=1 stream=1 REQUEST_ERROR error_code=0x10
[rust-sub] Error: SUBSCRIBE rejected: error_code=0x10 retry_interval=0 reason=""
```

moqtail-ts 0.13.0 copies the parameters of the SUBSCRIBE into its SUBSCRIBE_OK (`handlerSubscribe` in `dist/client-BzncAN14.mjs` of the npm package: `const parameters = [...msg.parameters]`). The relay's SUBSCRIBE carries FORWARD (§9.2), so the SUBSCRIBE_OK does too. §10.2.12 lists the messages FORWARD may appear in, "SUBSCRIBE, REQUEST_UPDATE (for a subscription), PUBLISH, PUBLISH_OK and SUBSCRIBE_TRACKS", and §10.2.1 says: "If it appears in some other type of message, the receiving endpoint MUST close the connection with a PROTOCOL_VIOLATION." The relay did that, and the subscriber got `DOES_NOT_EXIST`. The check is kept as it is.

`mise run check` ended with `OK` for all four parts. `mise run interop`: `32 of 32 pairings ended as expected`, after the change to what a FIN means in the relay and in both clients. None of those 32 pairings routes by namespace: every client in them sends PUBLISH for its track.

### What is decided where the draft leaves a choice

In [implementation notes](../moqt-implementation-notes.md), section Relay: which publisher is asked, what the relay's SUBSCRIBE carries, what the subscribers get for each answer of the publisher, and when the relay cancels its own SUBSCRIBE.

The scenes are tests of `xmoq-relay-core` (`src/tests/routing.rs`), as tables of input and expected output: 14 rows for which publisher gets the SUBSCRIBE (no match, the same namespace, a prefix, a namespace without fields, a longer namespace, a field that differs, two publishers, a withdrawn one, a FIN that is not a withdrawal, a publisher that left, a publisher that only sent PUBLISH, the subscriber's own namespace), and 7 rows for what the subscribers get (SUBSCRIBE_OK, REQUEST_ERROR, a long reason, REDIRECT, a stream that ends unanswered, no answer in time, a publisher that leaves). The crossing PUBLISH has a test of its own there, with what still arrives for the cancelled SUBSCRIBE (its answer either way, the end of its stream, its timer). Nine tests run through the relay's I/O on the in-memory connection.

### What a FIN means

A requester that finishes its direction of a request stream and does nothing else has not cancelled the request. The relay used to end a subscription on the subscriber's FIN, and both clients ended a subscription they publish on the subscriber's FIN; all three now go on until STOP_SENDING, a reset, or the end of the session. The moqtail clients do not use a FIN to unsubscribe (moqtail-ts resets and stops the stream; the moqtail Rust client of these runs never unsubscribes), so no peer of the interop runs depended on the old reading.

Whether the moqtail relay finishes the stream of a PUBLISH before PUBLISH_DONE: the own Rust client now prints a line when its subscriber does, and it printed none in the 3 pairings that have it publish through the moqtail relay. These are short runs with one Object.

## Limitations and blockers

No blocker.

- **No Object was routed from a publisher written by someone else.** The relay's SUBSCRIBE was answered successfully by `xmoq-moqt-client` only. moqtail-ts answers a SUBSCRIBE for a track it registered, and run 10 tried it: the exchange ends at its SUBSCRIBE_OK, for the reason above. So a reading of §9.5 that the relay and the own client share and get wrong would still not show. What run 10 does show with a foreign peer: the relay's SUBSCRIBE was decoded and accepted by moqtail-ts. The subscriber side, which gets the relay's SUBSCRIBE_OK and Objects, was moqtail's in runs 2 to 4 and 7 to 9. A publisher on the `moqtail` crate (Rust) that answers SUBSCRIBE looks feasible (the crate has the message types and the stream handler the relay itself uses) and was not written; whether it copies the parameters as moqtail-ts does was not read.
- **The crossing of runs 6 to 9 was made with delays in the own publisher**, not met by chance, and not with a moqtail publisher: the moqtail Rust client of these runs announces no namespace, and moqtail-ts answers the relay's SUBSCRIBE at once (run 10). That the old code refused the PUBLISH in this order was read from the code and not run.
- **One publisher per track.** With several publishers of a matching namespace, only the first is asked, where §9.5 and §9.3 have the relay ask all of them and merge their Objects. Listed in the implementation notes as MUSTs that are not met, and in the roadmap.
- The own TypeScript client was not run as the subscriber of a routed track. The script of these runs drives moqtail-ts as its page; the own page is in `interop/`, which was not changed.
- These runs are a script under `spikes/`, not part of `mise run interop` or CI.
- A publisher that does not answer, answers with an error, or leaves while subscribers wait was exercised in tests on the in-memory connection, not with a real peer.
- One Object per run, loopback, one machine, Chromium only.

## Open questions raised

1. **When are the routed pairings added to the interop matrix?** They need a publisher kind in `interop/run.ts`. The crossing order of runs 6 to 9 belongs there as well.
2. **Should a subscriber's AUTHORIZATION_TOKEN reach the publisher behind the relay?** Today the relay copies nothing of the downstream SUBSCRIBE upstream. This follows the authentication decision.
3. **What does a subscriber get when the track's publisher cancels the relay's SUBSCRIBE and another publisher of the namespace is still there?** Today the track ends for the subscribers. With several publishers per track this changes.
4. **Is the FORWARD in moqtail-ts's SUBSCRIBE_OK reported upstream?** Not done from this project so far.
