# What the moqtail relay answers to a FETCH, and the bytes it answers with

- Date: 2026-10-11
- Commit SHA: not fixed when this was written. Built on `55c0d370`
- Roadmap item: Phase 1, FETCH and Joining FETCH in the Rust and the TypeScript session, against peers. The first step: fixtures, and the encoding in Rust

## Objective

The own implementation has no FETCH. Before one is written, take real bytes of FETCH, FETCH_OK and a fetch stream from the peer there is, the moqtail relay 0.15.0 with moqtail's test client, as the other fixtures were taken; decode them by the reading of draft 18 that the Rust encoding is written from; and see what that relay does with a FETCH, since the interop pairings will run against it.

## Environment

macOS 14.4 (23E214), arm64, loopback only, port 48600. The relay and the test client of `moqtail/moqtail` at tag `relay@0.15.0` (`moqtail/0.15.0+dd8623d`), built into `spikes/moqtail/.tools` as the fixtures README says. `spikes/fetch-capture`, new: it writes the bytes it is given on a control stream and on one request stream each, and prints what comes back on every stream. It encodes and decodes nothing; it uses `xmoq-moqt-io` for the connection.

## Commands

The relay, and the test client as a publisher of (`xmoq`, `phase0`) / `object`: 12 Groups, an Object every 250 ms, 8 bytes each, with 3 Objects a Group unless said otherwise. Every FETCH was sent 4 seconds after the publisher started, so while it was publishing.

```sh
spikes/moqtail/.tools/bin/relay --port 48600 --host 127.0.0.1 \
  --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem --log-folder <dir>
spikes/moqtail/.tools/bin/client --command publish --server moqt://127.0.0.1:48600 --no-cert-validation \
  --namespace xmoq/phase0 --track-name object --group-count 12 --interval 250 --payload-size 8 --objects-per-group 3

# 1 and 2: the test client asks, and logs the control messages.
spikes/moqtail/.tools/bin/client --command fetch ... --start-group 0 --start-object 1 --end-group 1 --end-object 2
spikes/moqtail/.tools/bin/client --command subscribe ... --joining-fetch --joining-start 0 --duration 2

# 3 to 11: the capture tool sends bytes, one run each. N is the track name.
C="spikes/fetch-capture/target/debug/xmoq-fetch-capture --url https://127.0.0.1:48600 --cert-sha256 <sha256> --control af000000"
N="02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74"
S="$(cat fixtures/moqt-18/subscribe-moqtail-rs.hex)"
$C --request "$(cat fixtures/moqt-18/fetch-standalone-moqtail-rs.hex)"   # 3
$C --request "$S" --request "16 00 07 02 02 00 01 01 20 c8"              # 4: Relative Joining, Joining Start 1
$C --request "$S" --request "16 00 07 02 03 00 02 01 20 c8"              # 5: Absolute Joining, Joining Start 2
$C --request "16 00 1b 00 01 $N 00 00 04 00 00"                          # 6: {0,0} to {4,0}; publisher with 2 Objects a Group, --object-id-step 2 --group-id-step 2
$C --request "16 00 1b 00 01 $N 01 00 01 00 00"                          # 7: {1,0} to {1,0}
$C --request "16 00 1d 00 01 $N 00 00 02 00 01 22 02"                    # 8: {0,0} to {2,0}, GROUP_ORDER Descending; 2 Objects a Group
$C --request "16 00 1b 00 01 $N 64 00 65 00 00"                          # 9: {100,0} to {101,0}; 2 Objects a Group
$C --request "16 00 05 00 02 08 00 00"                                   # 10: Relative Joining for Request ID 8, which is no subscription
$C --request "16 00 1a 00 01 02 04 78 6d 6f 71 06 70 68 61 73 65 30 05 6f 74 68 65 72 00 00 01 00 00"  # 11: the track `other`
```

**The requests of 4 to 11 were written by hand.** A test of `crates/moqt` decodes those of 4 to 10 and compares them with what was meant. That of 9 was first written with a two-byte variable-length integer of QUIC for the Group 100 (`40 64`), which draft 18 reads as Group 64 and Object 100 (§1.4.1 has another form); the relay then closed the request stream without an answer, and that was first taken for its answer to a range beyond the largest Object. The run was repeated with the bytes above.

## Observed results

**The fixtures were taken, and they decode by the reading of the draft.** In five places the relay did something else than draft 18 has. Each is one run.

### The bytes

| Taken | From run | Decodes to |
|---|---|---|
| FETCH, Standalone | 1 | Request ID 0, {0, 1} to {1, 2}, SUBSCRIBER_PRIORITY 200 |
| FETCH, Relative Joining | 2 | Request ID 2, Joining Request ID 0, Joining Start 0, SUBSCRIBER_PRIORITY 200 |
| FETCH_OK | 1, 2 | End Of Track 0, End Location {1, 2} and {6, 2}, no parameters, no Track Properties |
| A fetch stream of four Objects | 3 | {0, 1}, {0, 2}, {1, 0}, {1, 1}: Serialization Flags 0x1F, 0x01, 0x0D, 0x01 |
| A fetch stream of thirteen Objects, five Groups | 5 | Objects 0 to 2 of Groups 2 to 5, and {6, 0} |
| A fetch stream with gaps | 6 | {0, 0}, {0, 2}, {2, 0}, {2, 2}: flags 0x25 and 0x2D, with the properties Prior Object ID Gap and Prior Group ID Gap. A gap is an ID that is left out; no End of Range indicator |
| A fetch stream without Objects | 7 | FETCH_HEADER and a FIN |

The Rust decoder gives these values for the eight files, and the Rust encoder writes the same bytes from them (the tests of `xmoq-moqt` pass). The four streams are also read byte by byte; the four messages are read whole. The flags and the arithmetic of the Group ID and Object ID are as §11.4.4.1 has them.

### Where the relay differs from draft 18

| | Run | Draft 18 | What came back |
|---|---|---|---|
| a | 2, 4 | The range of a Joining FETCH ends at the Joining Location, the Largest Location "communicated in SUBSCRIBE_OK" (§5.1, §10.12.2.1), so that fetch and subscription are "contiguous and non-overlapping" | Run 4: SUBSCRIBE_OK with LARGEST_OBJECT {5, 2}; FETCH_OK with End Location {6, 1}; the fetch stream had Group 5 and {6, 0}, where Joining Start 1 from {5, 2} is Groups 4 and 5. {6, 0} came on the fetch stream and on the subscription's stream. **It looks as if the range is counted from the largest Location at the time of the FETCH.** The relay's `fetch_handler.rs` reads `track.largest_location` there and no Location kept with the subscription. In run 2 the two were the same Group, and the End Location was the Joining Location plus one Object |
| b | 6, 7 | "A Location.Object value of 0 means the entire group is requested" (§10.12.1) | Run 7, Start {1, 0} and End {1, 0}: FETCH_OK with End Location {1, 0} and a fetch stream without Objects, where Group 1 had three. Run 6, End {4, 0}: nothing of Group 4. **A Group named with Object 0 as the end is left out** |
| c | 9 | "If ... Start Location is greater than the Largest Object ... the publisher MUST return REQUEST_ERROR with error code INVALID_RANGE" (§10.12.3) | FETCH_OK with End Location {101, 0}, the one asked for, and a fetch stream without Objects |
| d | 10 | `INVALID_JOINING_REQUEST_ID` for a Joining Request ID that is no subscription of the session (§10.12.2) | REQUEST_ERROR `DOES_NOT_EXIST` (0x10), "Track does not exist". In `fetch_handler.rs`, a subscription that is not found leads to the same answer as a track that is not found |
| e | 8 | "A publisher MUST send fetched groups in the requested group order" (§10.12.3) | FETCH_OK with End Location {2, 0}; the fetch stream had {0, 0} and {0, 1}, the two Objects of Group 0, and was still open 2 seconds later. Nothing of Group 1 came, so the order of Groups was not seen |

Run 11, a track nobody published: REQUEST_ERROR `DOES_NOT_EXIST`, as the draft has it.

**Which of b, c and e is the relay's doing is not known.** The relay passes a FETCH on to the publisher where it cannot answer from what it holds (`fetch_handler.rs`, "An upstream FETCH issued before FETCH_OK to learn the End Location"), and the publisher was moqtail's test client, which answers a FETCH itself. The relay's own function for the range (`resolve_standalone_fetch_range`) takes Object 0 for the whole Group and refuses a Start beyond the largest Object with `INVALID_RANGE`, which is not what came back. a and d are in the relay's code.

## Limitations and blockers

No blocker.

- **One run each.** a depends on what the publisher had sent when the FETCH arrived.
- For b, c and e it is not known whether the relay or moqtail's test client as the publisher decides; the two were not run apart.
- **No fetch stream in Descending Group Order and none with an End of Range indicator was obtained.** The Rust tests for those read bytes written by hand from §11.4.4 (`fixtures/moqt-18-by-hand/`), where the reader of the draft and the writer of the expectation are the same. The reading of an indicator's two fields as a Location, and not as deltas, is provisional until another implementation's bytes are seen; the moqtail crate reads them so as well, and differs in what follows an indicator (implementation notes).
- The moqtail npm package and a browser were not run.
- The own relay and the own clients do not handle a FETCH yet: the request is decoded and refused with `NOT_SUPPORTED`, and a fetch stream is an unsupported stream type.
