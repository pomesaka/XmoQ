# The relay raises the Largest Object at an Object's first byte

- Date: 2026-10-25
- Commit SHA: not committed; the working copy on top of f2922637
- Roadmap item: none of its own; the known gap "The Largest Object of a track is raised when an Object has been read whole, in the relay" of `docs/moqt-implementation-notes.md`, which this change closes

## Objective

Check, against real WebTransport sessions, that the own relay counts an Object as received from its first byte (draft 18 §5.1.2, §10.2.11), and what follows from that for a viewer that comes while an Object is on its way: the LARGEST_OBJECT of its SUBSCRIBE_OK, where its subscription starts, and its Joining FETCH, which the relay holds until the Object is whole (§9.4) or asks of the publisher when the Object does not arrive whole.

## Expected (written before the runs)

The publisher, `xmoq-quiet-peer --quiet half-object`, sends Object 0 of Group 0 whole, then the first half of Object 0 of Group 1 (512 KiB), and the rest 2 seconds later (`--then finish`), followed by Object 1 of 16 bytes; or it resets the stream there and sends Object 0 of Group 2 (`--then reset`). The viewer subscribes while the Object is half there and sends a Relative Joining FETCH from the Group of its Joining Location (Joining Start 0).

- The relay holds the FETCH: `HOLD_FETCH_FOR_OBJECT session=1 stream=S wait_ms=5000`.
- `--then finish`, without a filter, with the Largest Object filter, from the own Rust client and from an own page: FETCH_OK with End Location {1, 1}, so the Joining Location is {1, 0}; the fetch stream has {1, 0} alone, 524,288 bytes, written by the relay from its cache (`WRITE_FETCH_STREAM … pieces=1`), with no FETCH to the publisher; the answer comes after the rest, not before 1 second from the first half; the subscription brings {1, 1} and not {1, 0}.
- `--then reset`, own Rust client without a filter: the relay asks the publisher for the range (`OPEN_REQUEST session=0 own=N FETCH`), which refuses with DOES_NOT_EXIST; the client is refused with 0x10; the subscription brings {2, 0} and not {1, 0}.

## Environment

macOS 14.4 (arm64), Rust 1.99.0, Bun 1.3.14, the Chromium of Playwright that `interop/` installs. Own relay (`xmoq-relay`), own Rust client (`xmoq-moqt-client`), own page (`@xmoq/moqt`) and `xmoq-quiet-peer`, all from the working copy; MoQ Transport draft 18 over WebTransport on the loopback interface, with the certificate of `interop/`.

## Commands

```
cargo build --workspace
cargo build --locked --release -p xmoq-watch -p xmoq-quiet-peer -p xmoq-moqt-client -p xmoq-relay
cd interop
bun half-object.ts --case rust-finish|rust-largest|rust-reset|page-finish --port 24390   # each 3 times
bun matrix.ts --only "fetch" --base-port 25100
bun matrix.ts --only "own relay: video" --base-port 25100
```

## Observed results

- The first runs failed for the driver and the peer, not the relay: the peer published under `(xmoq)` and the viewer asked under `(xmoq, phase0)`, so the relay refused the SUBSCRIBE with DOES_NOT_EXIST; and the driver of `rust-reset` waited for `SEND … REQUEST_ERROR` without its `error_code=0x10`. Both were mended, and the runs below are with the mended ones.
- `half-object.ts`, 3 runs of each of the 4 cases: 12 of 12 as expected.
  - `rust-finish`, `rust-largest`: FETCH_OK End Location {1, 1}, `FETCH_OBJECT … group=1 object=0 … payload_len=524288`, `FETCH_ENDED … how=fin … objects=1`, and `OBJECT group=1 object=1` alone on the subscription. The FETCH_OK reached the client 2004 to 2010 ms after the first half was sent (the hold was 2000 ms). The relay printed `HOLD_FETCH_FOR_OBJECT session=1 stream=2 wait_ms=5000`, then `SEND session=1 stream=2 FETCH_OK` and `WRITE_FETCH_STREAM session=1 request_id=2 pieces=1` after the peer's `HALF_OBJECT_ENDED how=finish`, and no `OPEN_REQUEST` FETCH.
  - `page-finish`: the same, the page's FETCH over 2098 to 2109 ms after the first half; its subscription brought Object 1 of Group 1 alone.
  - `rust-reset`: `HOLD_FETCH_FOR_OBJECT`, then, after `HALF_OBJECT_ENDED how=reset`, `OPEN_REQUEST session=0 own=0 FETCH`, the peer's REQUEST_ERROR 0x10 passed on (`FETCH_REFUSED request_id=2 error_code=0x10`), and `OBJECT group=2 object=0` alone on the subscription.
- `matrix.ts --only "fetch"`: 35 of 35 as expected, the 4 runs above among them.
- `matrix.ts --only "own relay: video"`: 12 of 12 as expected. No run of that group printed `HOLD_FETCH_FOR_OBJECT`: the Objects of the video runs arrive whole within a read on the loopback interface, as before.

## Limitations and blockers

- Not run: the whole matrix (`mise run interop`). The relay now gives other LARGEST_OBJECT values where an Object is under way, which the other runs could meet.
- The hold's time limit (`ReceiptTimeout`, 5 seconds) is tested in the relay core alone; no interop run holds an Object back that long.
- The page case is without a filter only; a page with a filter was not run.

## Open questions raised

- The 5 seconds of the hold are provisional (`docs/open-questions.md`).

After that, the workspace's `target/` was cleaned (the disk of the machine had run full) and everything built again: `mise run check` passed, and `mise run interop -- --base-port 45300` ended 212 of 212 as expected (428 s). Then the change was put on `main` with #316 to #321 under it (four files had conflicts, each of two additions side by side, kept both); `mise run check` passed, `bun interop/matrix.ts --only "relay receives an Object" --base-port 31500` ended 3 of 3 and `--only "fetch:"` 32 of 32, and `mise run interop -- --base-port 31500` 212 of 212 (434 s). Then it was put on `main` with #323 under it (a stream of a Subgroup taken from another is read for a check alone): what begins on such a stream raises nothing and is compared when it is whole; `mise run check` passed, `--only "relay receives an Object"` ended 3 of 3, `--only "leaving:"` 38 of 38 (once the debug binaries were built again), and `mise run interop -- --base-port 31500` 218 of 218 (431 s).
