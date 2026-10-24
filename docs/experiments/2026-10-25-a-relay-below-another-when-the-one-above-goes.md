# A relay below another, when the one above goes

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale; the second step of building the own relay into an edge of the [design draft](../scale-and-meetings.md) (losing the upstream), with RENDEZVOUS_TIMEOUT through it

## Objective

After [a relay below another relay](2026-10-25-a-relay-below-another-relay.md): does a page that plays through the own relay below go on when the relay above is stopped and started anew; does a SUBSCRIBE that comes before the broadcast at the relay below wait there and at the relay above (the relay below asks with a RENDEZVOUS_TIMEOUT of its own); and does a subscriber of the relay below go on, with no PUBLISH_DONE, when the relay above sends GOAWAY naming another?

## Expected

What the runs check (written into the drivers and the tests before the first run; this list was written after it):

- `video.ts --video reconnect-through-edge`: as `reconnect`, with the viewer at the relay below. Both pages are told that what they had ended, fail while the relay above is gone, and go on: the publisher on a second session, the viewer showing 25 frames on a second session.
- `rendezvous.ts --case publisher-comes --through-edge`, with own-browser and with own-rust: the relay above holds a SUBSCRIBE of its session 0, the relay below, and answers nothing for 2 s; then own-rust publishes the track at the relay above, the held SUBSCRIBE is accepted at both, and the subscriber gets the Object.
- The relay's test `a_subscriber_of_the_edge_goes_on_when_the_origin_sends_it_elsewhere` (in-memory connections): the relay above sends GOAWAY naming another, the relay below takes the track from there as well, and when the first is gone the subscriber gets the next Group and no PUBLISH_DONE.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 of Playwright; both relays the own one (`xmoq-relay/0.0.0`), the one below on the run's port plus 1,000; `mise run interop -- --base-port 34500 --only "edge:"`; `cargo test -p xmoq-relay upstream::`.

## Result

**As expected, all three runs and the test**, in the first run of each:

| Run | Result |
|---|---|
| reconnect through the relay below | the publisher published on a second session after 9 tries that failed, and the viewer showed 28 frames before and 25 after, after 2 tries that failed (14.8 s); in a later run 10 tries, 17.2 s |
| rendezvous, own-browser at the relay below | held at both, accepted when own-rust published, the Object came (2.8 s) |
| rendezvous, own-rust at the relay below | the same (2.3 s) |
| the GOAWAY, in the relay's test | the subscriber got Group 1 from the relay named, after the first was stopped, and no PUBLISH_DONE |

The relay below in the run with the relay above stopped:

```
CONNECT_UPSTREAM uri=
UPSTREAM_CONNECTED session=1
CONNECT_UPSTREAM uri=
SEND session=0 stream=1 PUBLISH_DONE status_code=0x2 stream_count=2
UPSTREAM_FAILED connection lost: no session with 127.0.0.1:34506 within 10s, ...
CONNECT_UPSTREAM uri=
UPSTREAM_CONNECTED session=4
```

The relay above, stopped with SIGINT, sends its GOAWAY with no URI first (the harness gives it a Timeout of 0), and the relay below connected at once to the same place, as §10.4 has for a GOAWAY without one; that attempt failed after the 10 seconds of a connect, while the relay above was going and then gone. The viewer was told PUBLISH_DONE (status `0x2`) in that time. Why the attempt took the 10 seconds and was not refused at once was not looked at.

The two relays in the run with own-rust as the subscriber:

```
[relay:second] HOLD_REQUEST session=0 stream=1 SUBSCRIBE
[relay:second] CONNECT_UPSTREAM uri=
[relay:second] UPSTREAM_CONNECTED session=1
[relay:second] OPEN_REQUEST session=1 own=0 SUBSCRIBE
[relay] HOLD_REQUEST session=0 stream=1 SUBSCRIBE
[relay] SEND session=0 stream=1 SUBSCRIBE_OK track_alias=0
[relay:second] RECV session=1 own=0 SUBSCRIBE_OK track_alias=0
[relay:second] SEND session=0 stream=1 SUBSCRIBE_OK track_alias=0
```

## Not looked at

- A GOAWAY through real connections and processes: the interop run has no third relay (one port a run, and one 1,000 above it), so it is in the relay's tests alone, on in-memory connections.
- A GOAWAY on the stream of a request of the relay below, and not on the control stream (not taken; known gap).
- How long the viewer's picture stood still over the stop, and the delay through two relays.
- moqtail's relay above with RENDEZVOUS_TIMEOUT: the drivers read the own relay's lines.
