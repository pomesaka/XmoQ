# The LOC Timestamp inside Immutable Properties, in Rust

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: none closed. The Rust half of the known gap that `@xmoq/media` filled ([record](2026-10-24-the-loc-timestamp-inside-immutable-properties.md)).

## Objective

Transport §12.7: "When looking for the value of a property, processors MUST search both the mutable properties and the contents of Immutable Properties." The Rust client (`crates/moqt-client/src/video.rs`) and the terminal viewer (`crates/watch/src/bin/xmoq-watch/session.rs`) gave `read_timing` of `xmoq-loc` the properties of an Object as they came. Now they give it `searched_properties` of `xmoq_moqt::immutable` with `Of::Object` (#310, which the relay uses as well): the properties outside, then those inside, and those outside alone where the inside cannot be opened, as `locProperties` of `@xmoq/media` now does too. `xmoq-loc` is unchanged.

## Expected

Written before the test was run: the four rows of a test of the function this change first added (outside alone, inside alone, outside and inside, an Immutable Properties inside another) hold; the interop runs, whose publishers write the Timestamp outside, end as before.

## Environment

macOS 14.4, arm64; the working copy on `main` at `mwtwklwz`; `cargo test -p xmoq-moqt`; `mise run check`.

## Result

- The test passed, and `mise run check` passed. `bun interop/matrix.ts --only "watch:" --base-port 45300` ended 4 of 4 as expected, and `mise run interop -- --base-port 45300` 205 of 205 (436 s).
- **Not run**: an Object with its Timestamp inside Immutable Properties from another implementation, or through a relay, to the Rust client or the terminal viewer.
- After the review, the change was put on `main` with `searched_properties` (#310), which does the same: the function this change had added, and its test, were taken out, and the callers use `searched_properties`; its tests read the rows of `fixtures/immutable-properties/`. `locProperties` of `@xmoq/media` gives the properties outside alone where the inside cannot be opened, as `searched_properties` does (it gave them as they came before; the time read is the same, as both readers of LOC pass over every Type but the Timestamp and the Timescale). After that, `mise run check` passed, `bun interop/matrix.ts --only "watch:" --base-port 45300` ended 4 of 4 and `mise run interop -- --base-port 45300` 206 of 206 as expected (422 s).
