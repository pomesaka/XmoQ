# A relay below another relay

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale (decided on 2026-10-25: one relay serves 500 viewers of a broadcast, and relays are added to build a broadcast of 10,000); the first step of choice B and choice 1 of the [design draft](../scale-and-meetings.md)

## Objective

The own relay is given another relay as its upstream (`--upstream`, `--upstream-cert-sha256`, `--upstream-namespace`) and asks it for a track no session of its own announced, opening its session with it when a request needs one (decided on 2026-10-25). Does a page's Object reach subscribers at the relay below, through the own relay and through `moqtail`'s above it, with one session between the two relays for all the subscribers?

## Expected

What the pairings check (written into `interop/matrix.ts` before the first run; this list was written after it):

- Two own-browser subscribers at the relay below get the Object a page publishes at the relay above, and then PUBLISH_DONE when the page leaves: it passes both relays.
- A page that only announces the namespace at the relay above is asked for the track by that relay, after the relay below asked it, and own-rust at the relay below gets the Object.
- `moqtail`'s Rust client at the relay below gets the Object.
- The relay below opens one session with the one above in each run (`UPSTREAM_CONNECTED` once), whatever the number of subscribers.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 of Playwright; the relay above the own one (`xmoq-relay/0.0.0`) or `moqtail/0.15.0+dd8623d`; the relay below the own one, on the run's port plus 1,000; `mise run interop -- --base-port 34500 --only "edge:"`.

## Result

**As expected, all six** (three pairings under each relay above), in the first run:

| Relay above | Pairing | Result |
|---|---|---|
| `moqtail` | own-browser to 2 own-browser at an own relay below, then PUBLISH_DONE | the payload matched, 1.6 s |
| `moqtail` | own-browser only announces the namespace, and serves own-rust at an own relay below | the payload matched, 0.9 s |
| `moqtail` | own-browser to moqtail-rust at an own relay below | the payload matched, 1.0 s |
| own | own-browser to 2 own-browser at an own relay below, then PUBLISH_DONE | the payload matched, 1.1 s |
| own | own-browser only announces the namespace, and serves own-rust at an own relay below | the payload matched, 0.6 s |
| own | own-browser to moqtail-rust at an own relay below | the payload matched, 0.6 s |

`UPSTREAM_CONNECTED` is in each of the six logs once. The relay below, with two subscribers (the own relay above):

```
HOLD_REQUEST session=0 stream=1 SUBSCRIBE
CONNECT_UPSTREAM
UPSTREAM_CONNECTED session=1
OPEN_REQUEST session=1 own=0 SUBSCRIBE
RECV session=1 own=0 SUBSCRIBE_OK track_alias=0
SEND session=0 stream=1 SUBSCRIBE_OK track_alias=0
SEND session=2 stream=1 SUBSCRIBE_OK track_alias=0
RECV session=1 own=0 PUBLISH_DONE status_code=0x2 stream_count=1
SEND session=0 stream=1 PUBLISH_DONE status_code=0x2 stream_count=1
SEND session=2 stream=1 PUBLISH_DONE status_code=0x2 stream_count=1
```

The second subscriber came after the session was open and was answered from the track the relay below already had.

## Not looked at

- One Object a run, on one machine: no video, no delay measured through two relays, and nothing about load.
- A viewer that comes before the broadcast at the relay below: the relay below asks the upstream without RENDEZVOUS_TIMEOUT (known gap in the [implementation notes](../moqt-implementation-notes.md), row on a relay above this one).
- A FETCH or TRACK_STATUS at the relay below that comes while it has no session with the upstream, which is refused (known gap); the Joining FETCH of a catalog after a SUBSCRIBE was not run.
- The chat through two relays, which needs PUBLISH_NAMESPACE passed upstream (not built), and `--auth`.
- The relay below closing its session after 30 seconds without anything held, and opening one again, is run in the relay's tests on in-memory connections only (`an_edge_that_held_nothing_connects_again_for_its_next_subscriber`).
