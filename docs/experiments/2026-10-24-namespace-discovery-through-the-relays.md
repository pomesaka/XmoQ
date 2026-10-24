# Namespace discovery through the relays

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, chat over MoQ tracks (Q36, decided on 2026-10-24): the gatherer of the chat finds the participants' tracks by SUBSCRIBE_NAMESPACE. Also a known gap of the implementation notes ("Messages the relay does not implement").

## Objective

Draft 18 §6.1: "SUBSCRIBE_NAMESPACE requests namespace discovery: the publisher sends relevant NAMESPACE and NAMESPACE_DONE messages for namespaces matching the prefix". The relay answered SUBSCRIBE_NAMESPACE with `NOT_SUPPORTED`, and neither implementation decoded NAMESPACE or NAMESPACE_DONE. Now `xmoq-moqt` sends and takes all three, the relay serves SUBSCRIBE_NAMESPACE (`crates/relay-core/src/discovery.rs`), `@xmoq/moqt` decodes and encodes them, and the own Rust client has a mode that asks (`--mode subscribe-namespace`).

## Expected

**Written after the runs**, which were made while the change was being written; what was held to is the draft's text, and the bytes of another implementation's library.

- The byte rows of moqtail npm 0.13.0 (`fixtures/moqt-18-from-libraries/subscribe-namespace-*`, `namespace-*`) decode to the values of their README in both implementations, and are encoded to the same bytes.
- Through the own relay and through `moqtail`'s: the watcher is answered (`NAMESPACES_SUBSCRIBED`), told `NAMESPACE namespace=xmoq/phase0` when an own-rust publisher announces (`xmoq`, `phase0`), and `NAMESPACE_DONE namespace=xmoq/phase0` when that publisher leaves.

## Environment

macOS 14.4, arm64, loopback; the working copy on `main` at `sozltwsl`, and then at `twmsowsr`; `bun interop/namespaces.ts --relay own|moqtail --port 45390`; `mise run check`.

## Result

- Through the own relay: "SUBSCRIBE_NAMESPACE (xmoq) was answered, told "NAMESPACE namespace=xmoq/phase0" when the publisher announced and "NAMESPACE_DONE namespace=xmoq/phase0" when it left". Through `moqtail`'s relay (0.15.0): the same lines. The two relays write NAMESPACE with the suffix alone (§10.16), which the own client puts after its prefix.
- `mise run interop -- --base-port 45300`: 181 of 183 pairings ended as expected (289 s). The two that did not were the new ones: the matrix gave the driver `--relay none` after its own `--relay`, which the driver refused. With the matrix passing the pairing's relay instead, `bun matrix.ts --only namespaces:` ended 2 of 2 as expected. Then, on `main` with the end of track (`twmsowsr`) under the change, `mise run interop -- --base-port 45300` ended 184 of 184 as expected (292 s).
- `mise run check` passed, with the byte rows in both implementations, the tests of the Rust session (the answer, NAMESPACE and NAMESPACE_DONE, a NAMESPACE_DONE before its NAMESPACE, a NAMESPACE before the answer; five prefixes against an established one for `PREFIX_OVERLAP`) and of the relay's core (a namespace announced before and after, one outside the prefix, one two sessions announce, its withdrawal by one and by the other, the end of the request).
- After the review of the change, run in `mise run check`, not against another implementation: a REQUEST_UPDATE with TRACK_NAMESPACE_PREFIX is refused for a SUBSCRIBE_NAMESPACE and its stream closed, and closes the session for a SUBSCRIBE (Rust session; `@xmoq/moqt` for a SUBSCRIBE waiting for its answer, and no answer on the stream of a refused SUBSCRIBE_NAMESPACE); `@xmoq/moqt` closes the session for a SUBSCRIBE_NAMESPACE whose prefix has 33 fields or that carries FORWARD, and refuses one of 2 fields with `NOT_SUPPORTED`; the relay's core tells a watch of 100 namespaces 64 at first and the rest as they are reported written, refuses a watch with `NAMESPACE_TOO_LARGE` when 4,097 namespaces are announced under its prefix, and resets it with `EXCESSIVE_LOAD` when they grow from 4,096 to 4,097; the relay resets the stream of a SUBSCRIBE_NAMESPACE whose queue is full with `EXCESSIVE_LOAD` and keeps the session (a queue of 1); the Rust session ends a request named a 4,097th namespace and goes on. With these, and the limit of the relay at 128 namespaces told of at once, `mise run interop -- --base-port 45300` ended 187 of 187 as expected (309 s). The limit was then raised to 4,096, with the window of 64; after that, `mise run check` passed and `bun matrix.ts --only namespaces:` ended 2 of 2 as expected (through the own relay, the NAMESPACE now goes out as the I/O side reports the one before written); the whole matrix was not run again. The bytes of TRACK_NAMESPACE_PREFIX are written by hand from §10.2.14: the moqtail crate 0.15.0 writes them otherwise (the row in the implementation notes).
- **Not run**: a REQUEST_UPDATE of a SUBSCRIBE_NAMESPACE through a relay; a prefix of no field against `moqtail`'s relay; the limit of 8 watches for a session at the relay; any of the limits against another implementation.
