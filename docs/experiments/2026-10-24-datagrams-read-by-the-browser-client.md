# Datagrams read by the browser client

- Date: 2026-10-24
- Commit SHA: f349fbfc, the commit that adds this record
- Roadmap item: Phase 0b, the MoQT layer of the TypeScript implementation

## Objective

See that `@xmoq/moqt` reads the datagrams of its WebTransport session, closes the session with `PROTOCOL_VIOLATION` for a datagram whose type is not allowed, drops the others, and reads one datagram at a time without keeping any.

## What the draft says

§11: "All MOQT datagrams start with a variable-length integer indicating the type of the datagram", and "An endpoint that receives an unknown datagram type MUST close the session". §11.3.1: the Type of an OBJECT_DATAGRAM "takes the form 0b00X0XXXX"; Type values with both STATUS (`0x20`) and END_OF_GROUP (`0x02`) set, and values that do not match the form, are invalid, and the endpoint "MUST close the session with a PROTOCOL_VIOLATION". §11.5.2: a PADDING DATAGRAM has the type `0x132B3E29`, and "The receiver MUST discard all data received in a padding datagram". The rule the client keeps is in the row on a datagram of the peer in [the implementation notes](../moqt-implementation-notes.md).

## Expected

Written down after the runs, from what the change was asked to do; the tests that check it were written before they were run.

- The table-driven test of `classifyDatagram` passes: Types `0x00` to `0x0F` and `0x20`, `0x21`, `0x24`, `0x25`, `0x28`, `0x29`, `0x2C`, `0x2D` are OBJECT_DATAGRAMs, also in a non-minimal encoding; `0x132B3E29` is padding; `0x10` to `0x1F`, `0x30` and up, the type of a padding stream, the eight Types with STATUS and END_OF_GROUP, an empty datagram and one that ends inside its type are faults.
- Over the fake transport, a datagram of a type that is not allowed ends the session: the transport is closed with code 3 and the reason of the fault. An OBJECT_DATAGRAM and a PADDING DATAGRAM before it leave the session open, and the reading goes on to the one after them.
- The `quiet peer:` pairings of `interop/matrix.ts`, which now run with a page that reads datagrams in Chromium, all end as expected. The quiet peer sends no datagram, so they show only that reading does not disturb a session.

## Environment

macOS (Darwin 23.4.0), Bun 1.3.14, the Chromium of `interop/` (Playwright), TypeScript 6.0.3. Tree at the commit after `14a07c79` with this change.

## Commands

```
mise run check
bun interop/matrix.ts --only "quiet peer:" --base-port 45300
```

## Result

- `mise run check` passed. The tests of `@xmoq/moqt`: 1276 pass, 0 fail, across 30 files, with `test/core/datagram.test.ts` (37 rows) and the 4 rows on datagrams in `test/webtransport/connection.test.ts` among them.
- `interop/matrix.ts --only "quiet peer:"`: 31 of 31 pairings ended as expected, in 53.3 s. In the pairings of own-browser, the page took a reader of `datagrams.readable` from Chromium's `WebTransport` without an error.

## Not run

- No peer sent a datagram to the page in Chromium. The quiet peer (`crates/quiet-peer`, `interop/quiet-peer.ts`) has no mode that sends one in this tree; another PR adds it, and an own-browser row is to follow it.
- What Chromium does with datagrams that wait while the page does not read: the size of its queue and its age limit were not measured. The bound is read in the W3C WebTransport Editor's Draft, not seen.
- The Rust side. It reads no datagrams in this tree; another PR adds the same rule there.
- The full `mise run interop`.

## Next action

Once the quiet peer can send a datagram, add the pairing of own-browser to it and run it three times.

After the change was put on `main` with the Rust side and the datagram case of `xmoq-quiet-peer` (#285), own-browser was added to that case. Expected, written before the run: the peer prints `DATAGRAM_SENT session=0 type=0x30` and then `SESSION_ENDED session=0 ClosedByPeer { code: 3,`. `bun interop/matrix.ts --only "at a datagram of an unknown type" --base-port 45300` ended 3 of 3 as expected three times running (own-rust, the terminal viewer, own-browser), once the release binaries were built again; the runs before that failed for own-rust and the terminal viewer, with binaries built before #285. Then `mise run check` passed and `mise run interop -- --base-port 45300` ended 200 of 200 as expected (398 s).
