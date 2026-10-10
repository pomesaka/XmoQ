# A fetch stream of the moqtail relay, read by the own stream readers

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, FETCH in the Rust session and its streams

## Objective

The readers of `xmoq-moqt-io` now take a unidirectional stream of type 0x05: they read the FETCH_HEADER, announce the stream, and hand its bytes on to whoever takes them. Their tests use the in-memory connection. See that they take the fetch stream of a real peer over WebTransport the same way, and that nothing is lost or added on the way.

## Environment

macOS 14.4 (23E214), arm64, loopback only, port 48600. The relay and the test client of `moqtail/moqtail` at tag `relay@0.15.0`, as in [the record of the captures](2026-10-11-moqtail-relay-fetch.md). `spikes/fetch-capture` with the new flag `--readers on`, which reads the peer's unidirectional streams with `read_uni_streams` and prints each fetch stream when it has ended.

## Commands

The relay and the publisher as in the record of the captures (12 Groups of 3 Objects, 250 ms apart). Four seconds after the publisher started, twice:

```sh
C="spikes/fetch-capture/target/debug/xmoq-fetch-capture --url https://127.0.0.1:48600 --cert-sha256 <sha256> --control af000000"
$C --readers on --request "$(cat fixtures/moqt-18/fetch-standalone-moqtail-rs.hex)"
$C --request "$(cat fixtures/moqt-18/fetch-standalone-moqtail-rs.hex)"
```

## Observed results

**The readers announced the stream with Request ID 0 and handed on the bytes that the raw read shows after the header, ending with a FIN.** One run of each.

```text
IO fetch stream=1 request_id=0
IO body stream=1 end=Fin 1f 00 01 01 80 08 4d 3b c3 24 4c 84 57 2d 01 08 4d 3b c3 24 4c 84 57 2d 0d 00 00 08 4d 3b c3 24 4c 84 57 2d 01 08 4d 3b c3 24 4c 84 57 2d
RECV bidi 0 end=open 18 00 04 00 01 02 00
```

The raw read of the second run, and the fixture `fetch-stream-from-moqtail-relay`, have `05 00` followed by the same 46 bytes.

## Limitations and blockers

No blocker.

- One stream of 48 bytes (the header and 46 bytes), which arrives in one read. That a stream longer than the reader's buffers is handed on whole, and that a reader nobody takes from holds the sender back, is covered by the tests with the in-memory connection only.
- A reset, a stop and a stream that ends inside its header were not run against a real peer.
- Nothing decodes the bytes in this run, and no own endpoint sends a FETCH yet: the request was the captured bytes.
- **The tool that compares two versions of the relay core (`spikes/relay-core-diff`) could not compare this change, and no count of differences was taken.** It builds the core of the revision before against the `xmoq-moqt` of the revision after, and the session of that crate has new variants the old core does not match on. The tool now ends with `NOT COMPARED` and exit code 2 in that case, where it printed a line of zeros next to its failure. What it cannot see, then and in any change of this kind: the change to the session, and the lines of the core that only fit it to the new session. Those lines are held by a test of the core instead: a FETCH gets the same actions as a request the session refuses by itself (REQUEST_ERROR `NOT_SUPPORTED`, then a FIN).
