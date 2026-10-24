# GOAWAY and moqtail

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: interop with moqtail (GOAWAY of #248 and #251, the Request IDs of #256)

## Objective

GOAWAY and the checks of Request IDs that came to main today were run against `xmoq-quiet-peer` alone. Whether moqtail, in the versions the interop runs use, sends or takes GOAWAY, and where it sends one, the own clients against it.

## What was read

moqtail at the tag `relay@0.15.0` (commit `dd8623dacd69de2577ca08ba700efcb40546dc1a`), cloned on 2026-10-24 (the relay of `scripts/ci/interop.sh`, and the crate `moqtail` 0.15.0 of `interop/peers/moqtail/rust-client`); the npm package `moqtail` 0.13.0 of `interop/package.json`, read in `interop/node_modules/moqtail/dist/`.

| Peer | Sends GOAWAY | Takes GOAWAY |
|---|---|---|
| The relay (`apps/relay/src/server.rs`) | **Yes, at SIGTERM**: `broadcast_goaway` sends every connected client a GOAWAY on the control stream with the URI of `--redirect-uri` (none without the flag), a Timeout of 10,000 ms (`DRAIN_TIMEOUT_MS`) and Request ID 0, refuses new requests with `GOING_AWAY` from then on (`server/session.rs`), and exits 10 seconds later. Never at another time | Not looked into: the own clients send none |
| The browser client (npm 0.13.0, `src/client/handler/goaway.ts`) | No: `new GoAway(...)` is in its tests alone | Yes: on the control stream it closes the session with `PROTOCOL_VIOLATION` for a second one, and otherwise notes it and calls `onGoaway`; on a request stream it issues the request again (`migrateRequest`) |
| The crate (0.15.0) | No client of its own; `GoAway` encodes and decodes, with the Request ID optional and last | |

- The bytes the relay sent in the runs: `10 00 1b 17 <23 bytes of https://127.0.0.1:49701> a7 10 00`: type 0x10, length 27, the URI, Timeout 10,000, Request ID 0.
- Request ID 0 is the client's parity (§10.4: "If no requests have been processed, this is 0 (at a server)"). The own clients take it: they check its parity, and close for one of the sender's.
- The own relay sends no GOAWAY, so moqtail's taking of one was not run.

## Commands

```sh
cd interop
bun video.ts --video goaway-from-moqtail --port 48700
bun video.ts --video goaway-from-moqtail-rust --port 48701
mise run interop -- --base-port 48100
```

The driver starts the own relay on the run's port plus 1,000, then moqtail's relay on the run's port with `--redirect-uri https://127.0.0.1:<that port>`, and sends moqtail's relay SIGTERM once the clients play.

## Observed results

- **Two own pages move**: the publisher (`keepPublishing`) and the viewer (`keepPlaying`) each said `moved https://127.0.0.1:49700`, connected there and published and played on a second session, and the viewer showed 25 frames before and 25 after, in order.
- **The own Rust client** printed `GOAWAY uri=https://127.0.0.1:49701 timeout_ms=10000 request_id=0` and left with 0, having connected once.
- Both runs are in the interop matrix now (`moqtail relay: goaway: ...`).
- **The terminal viewer did not get that far through moqtail's relay**: `no catalog came within 5 s`, before any GOAWAY. It reads the catalog with a Relative Joining FETCH after its SUBSCRIBE (§10.12.2). The page's SUBSCRIBE_OK had Largest Object {1792818306369, 0} (the page numbers its Groups by the clock); moqtail's relay counted the range of the Joining FETCH from {0, 0} to {0, 1} and asked the page for Group 0 (start {0, 0}, end {0, 0}, which §10.12.1 reads as all of Group 0: "A Location.Object value of 0 means the entire group is requested"); the page has no Group 0, answered FETCH_OK with end {0, 0} and an empty stream, and moqtail's relay sent the viewer an empty one. §10.12.2.1 has the end of a Joining FETCH from the subscription's Joining Location, which is the page's Largest Object here. This is moqtail's, not a reading of the own side: the page answered what it was asked. The terminal viewer is left out of the GOAWAY runs for it.

### A viewer that comes to the new relay before the publisher

In CI the run of the two pages failed once (182 of 183, run 40065497457): the viewer moved, came to the own relay before the publisher and was refused (`REQUEST_ERROR 0x10`), and its next try went to moqtail's relay, the URL it was first given, which refused it (`0x6`, "Relay is draining") and then was gone. `keepPlaying` and `keepPublishing` handed the GOAWAY to the next `connect` alone; the page's `connect` falls back to its first URL without one.

- Changed: both hand the last GOAWAY to every `connect` until another comes (`packages/media/src/moqt/keep-broadcast.ts`). The terminal viewer had the same shape (its URL of a GOAWAY was for the next try alone) and now keeps to it for as long as its room or its flags name the place it left.
- A run that sets the order: `--video goaway-from-moqtail-viewer-first`, where the publishing page waits 3 s before it connects after the GOAWAY. With the change it passed 5 times of 5; the viewer failed at the own relay six times (every 0.5 s) and then played there. With the GOAWAY handed to the next `connect` alone (the code before, put back for one run) it failed as in CI: the viewer did not play on the new relay.
- The run without a set order passed 20 times of 20 with the change.

## Limitations

- moqtail's relay sends GOAWAY at SIGTERM alone; a GOAWAY on a request stream, and one at another time, were not run against moqtail.
- The pages and the own Rust client were run; the terminal viewer against moqtail was not (above).

## Decision or next action

None. No reading of the own side was found wrong.
