# Immutable Properties in the browser client

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: none of its own; draft 18 §12.7 (MUST) for `@xmoq/moqt`, the row "Immutable Properties" of the [implementation notes](../moqt-implementation-notes.md). The Rust implementation comes in another pull request.

## Objective

Draft 18 §12.7: "When looking for the value of a property, processors MUST search both the mutable properties and the contents of Immutable Properties", and "A Track is considered malformed (see Section 2.4.2) if ... An Object contains an Immutable Properties property that contains another Immutable Properties key [or] A Key-Value-Pair cannot be parsed." `@xmoq/moqt` now decodes the Key-Value-Pairs inside Immutable Properties (0xB) of an Object (subgroup and fetch streams) and of a Track (SUBSCRIBE_OK, PUBLISH, REQUEST_OK, FETCH_OK), and looks inside for what it checks. The question is whether that changes anything for the streams that real peers send, which carry no Immutable Properties.

## Expected, written before running

- `mise run check` passes, with the table `fixtures/immutable-properties/properties.tsv` (21 rows) read by `packages/moqt/test/core/immutable-properties.test.ts`.
- The interop rows of the browser client (`bun interop/matrix.ts --only own-browser`) pass as before: no peer sends Immutable Properties, so the change is not reached there, and what it touches on the way (the decoding of every Object's properties and of Track Properties) gives the same results.
- No run reaches Immutable Properties from a peer: neither `moqtail` 0.15.0 nor `moqlivemock` was made to send one.

## Environment

macOS 14.4, arm64, loopback; the working copy; Bun 1.3.14; Chromium of playwright-core as `interop/` pins it.

## Result

As expected.

- `mise run check` passed: the 1,311 tests of `@xmoq/moqt` among them, with the 21 rows of the table, a row each for the SUBSCRIBE_OK and the FETCH_OK whose Immutable Properties make the Track malformed (in the session, and in the connection: `RequestAbortedError` of `malformed-track`, the FETCH ended with `malformed-track` and its request stream reset with `MALFORMED_TRACK`, 0x12, the session open), and three rows of Objects the encoder refuses.
- `bun interop/matrix.ts --only own-browser --base-port 45300`, after `cargo build --workspace` and the release build of the four binaries: "95 of 95 pairings ended as expected in 111.0s".

## Not run

- Immutable Properties from another implementation: no capture holds one, and the table's bytes were written by hand from the draft (`fixtures/immutable-properties/README.md`).
- The Rust side reading the same table: another pull request.

After that, `mise run check` passed and `mise run interop -- --base-port 45300` ended 201 of 201 as expected.
