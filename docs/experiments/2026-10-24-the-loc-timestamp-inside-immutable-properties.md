# The LOC Timestamp inside Immutable Properties

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: none closed. A known gap of the implementation notes: the LOC properties of an Object were read from the mutable properties alone (transport §12.7, MUST).

## Objective

Transport §12.7: "When looking for the value of a property, processors MUST search both the mutable properties and the contents of Immutable Properties." `@xmoq/media` gave `readLocTiming` of `@xmoq/loc` the properties of an Object as they came, so a Timestamp inside Immutable Properties was not seen. Now `locProperties` of `packages/media/src/moqt/loc-properties.ts` opens Immutable Properties with `propertiesToSearch` of `@xmoq/moqt` (a new entry, `@xmoq/moqt/properties`, without I/O) before the video receiver and the audio player read the time. `@xmoq/loc` is unchanged: it reads the list it is given.

## Expected

Written before the test was run: a Timestamp outside is read as before; one inside Immutable Properties is read; one outside and one inside are two Timestamps, which `readLocTiming` takes for invalid ("the Timestamp comes twice"); an Immutable Properties that cannot be opened (one inside another) leaves the properties as they came. The interop runs, whose publishers write the Timestamp outside, end as before.

## Environment

macOS 14.4, arm64; the working copy on `main` at `xqysusnm`; `bun test packages/media/test/loc-properties.test.ts`; `mise run check`.

## Result

- `packages/media/test/loc-properties.test.ts`, five rows written by hand from §12.7 and LOC-04 (the Timestamp is `0x10`): all as expected. The first version of the rows wrote the value inside as `43 e8`, a varint of the form of QUIC, which draft 18's encoding does not read: `propertiesToSearch` said the pair could not be parsed and the Timestamp was not found. The rows were written again with a value of one byte, as the rows of `fixtures/immutable-properties/` are.
- `mise run check` passed, and `mise run interop -- --base-port 45300` ended 201 of 201 as expected (411 s), on `main` before the datagram pairings of the terminal viewer were added there.
- **Not run**: an Object with its Timestamp inside Immutable Properties from another implementation, or through a relay; the Rust side (`xmoq-loc`'s callers in the Rust client and the terminal viewer), which waits for the Rust side of Immutable Properties.
