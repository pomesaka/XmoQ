# A Standalone FETCH through the own relay

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

The own relay now passes a Standalone FETCH on to one publisher and that publisher's fetch stream back. See it do so with a publisher written by someone else, moqtail's test client, and see what it answers where it asks nobody: FILL_TIMEOUT 0, a track without a publisher, a Joining FETCH.

## Environment

macOS 14.4 (23E214), arm64, loopback only, port 48600. The own relay, built from the working copy this record is part of. moqtail's test client 0.15.0 as the publisher, from `spikes/moqtail/.tools`. `spikes/fetch-capture` as the subscriber: it sends the bytes it is given and prints what comes back, and encodes and decodes nothing; with `--readers on` it reads the relay's unidirectional streams with the readers of `xmoq-moqt-io`.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
spikes/moqtail/.tools/bin/client --command publish --server moqt://127.0.0.1:48600 --no-cert-validation \
  --namespace xmoq/phase0 --track-name object --group-count 12 --interval 250 --payload-size 8 --objects-per-group 3

# Four seconds after the publisher started, each against a relay and a publisher started anew.
C="spikes/fetch-capture/target/debug/xmoq-fetch-capture --url https://127.0.0.1:48600 --cert-sha256 <sha256> --control af000000"
N="02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74"
$C --request "$(cat fixtures/moqt-18/fetch-standalone-moqtail-rs.hex)"                 # 1: {0, 1} to {1, 2}
$C --readers on --request "$(cat fixtures/moqt-18/fetch-standalone-moqtail-rs.hex)"    # 2: the same
$C --request "16 00 1d 00 01 $N 00 00 02 00 01 0a 00"                                  # 3: {0, 0} to {2, 0}, FILL_TIMEOUT 0
$C --request "16 00 1a 00 01 02 04 78 6d 6f 71 06 70 68 61 73 65 30 05 6f 74 68 65 72 00 00 01 00 00"  # 4: the track `other`
$C --request "$(cat fixtures/moqt-18/subscribe-moqtail-rs.hex)" \
   --request "$(cat fixtures/moqt-18/fetch-relative-joining-moqtail-rs.hex)"           # 5: a Relative Joining FETCH
$C --request "16 00 1e 00 01 $N 00 00 02 00 01 0a 81 f4"                               # 6: {0, 0} to {2, 0}, FILL_TIMEOUT 500
$C --request "16 00 1d 00 01 $N 00 00 02 00 01 22 02"                                  # 7: the same range, GROUP_ORDER Descending

# 8 and 9: the publisher with --payload-size 1000000, so that the fetch stream is larger than the windows.
# The subscriber reads no unidirectional stream for 13 seconds. 8: --group-count 60. 9: --group-count 12.
$C --read-uni-after-ms 13000 --wait-ms 16000 --request "16 00 1b 00 01 $N 00 00 03 00 00"

# The relay core against the revision before, in the worlds without a FETCH.
python3 spikes/relay-core-diff/compare.py 7f264197 disk --world <world> --random-only
```

The requests of 3, 4 and 6 to 9 were written by hand; they are the ones of [the record of the captures](2026-10-11-moqtail-relay-fetch.md), some with a parameter added. `0a 81 f4` is FILL_TIMEOUT 500 in the variable-length integers of §1.4.1.

## Observed results

**The FETCH reached the publisher, and its answer and its fetch stream reached the subscriber.** One run each.

| Run | On the request stream | On the fetch stream | Relay log |
|---|---|---|---|
| 1 | `18 00 04 00 01 02 00`: FETCH_OK, End Location {1, 2}; then a FIN | `05 00`, then 45 bytes, then a FIN | `OPEN_REQUEST session=0 own=0 FETCH`, `SEND session=1 stream=1 FETCH_OK`, `PIPE_FETCH session=1 stream=1 request_id=0 from_session=0 from_stream=8`, `FETCH_PIPE_ENDED session=1 stream=1 finished` |
| 2 | the same | read by the readers of `xmoq-moqt-io`: Request ID 0, the same 45 bytes, a FIN | the same |
| 3 | `18 00 04 00 02 00 00`: FETCH_OK, End Location {2, 0}; then a FIN | `05 00 81 0c 02 ff ff ff ff ff ff ff ff ff`: End of Unknown Range at {2, 2^64 - 1}, then a FIN | `SEND session=1 stream=1 FETCH_OK`, `WRITE_FETCH_STREAM session=1 request_id=0 bytes=12`; no `OPEN_REQUEST` |
| 4 | `05 00 03 10 00 00`: REQUEST_ERROR `DOES_NOT_EXIST`; then a FIN | none | `SEND session=1 stream=1 REQUEST_ERROR error_code=0x10` |
| 5 | SUBSCRIBE_OK with LARGEST_OBJECT {5, 2} on the first stream; `05 00 03 03 00 00`, REQUEST_ERROR `NOT_SUPPORTED`, on the second | none | `SEND session=1 stream=2 REQUEST_ERROR error_code=0x3` |

**moqtail's publisher took the two parameters the relay passes on.** The session to the publisher stayed open in runs 6 and 7.

| Run | The relay's FETCH carries | On the request stream | On the fetch stream |
|---|---|---|---|
| 6 | FILL_TIMEOUT 500 | FETCH_OK, End Location {2, 0}; a FIN | Objects 0 to 2 of Groups 0 and 1; a FIN |
| 7 | GROUP_ORDER Descending | FETCH_OK, End Location {2, 0}; a FIN | Objects 0 to 2 of Group 0; a FIN |

What it sent for them is the publisher's own: nothing of Group 2 for an End Location {2, 0}, and for Descending the first Group alone. Run 6 was repeated to read the publisher's log: it had opened the streams of Groups 0 to 5 when the FETCH came, and logged "requested up to Location { group: 2, object: 0 }, answering up to Location { group: 2, object: 0 }" and "Served 6 objects". Those are two of the ways the moqtail relay's answers differed from the draft in the record of the captures (b and e), where it was not known whether the relay or the publisher behind it decided; here no moqtail relay is in between.

**A subscriber that read nothing lost its FETCH after 10 seconds, and a publisher that left took its FETCH with it.**

| Run | What happened | What the subscriber saw | Relay log |
|---|---|---|---|
| 8 | The subscriber read no unidirectional stream; the publisher kept publishing | FETCH_OK; then the request stream reset with 0x5 (`TOO_FAR_BEHIND`) | `PIPE_FETCH ...`, `FETCH_PIPE_ENDED session=1 stream=1 downstream_stalled`, `CANCEL_OWN_REQUEST session=0 own=0 code=0x1`, `RESET_REQUEST session=1 stream=1 code=0x5` |
| 9 | The same, and the publisher finished its 12 Groups and closed its connection while the fetch stream was stuck | FETCH_OK; then a FIN on the request stream | `PIPE_FETCH ...`, `CONNECTION_ENDED session=0 ClosedLocally`, `ABORT_FETCH_PIPE session=1 stream=1 code=0x6`, `FINISH_REQUEST session=1 stream=1` |

In both runs the subscriber's tool, when it began to read after 13 seconds, was not handed the fetch stream at all: the stream had been reset before it was accepted. The reset code of the fetch stream itself was therefore not seen in these runs.

The 45 bytes of runs 1 and 2 (counted again in a later run of 1; this record first had 46) are `1c 00 01 80 08 P 00 08 P 0e 00 00 08 P 01 08 P` with P = `4d 3b c3 24 4c 84 57 2d`: the Objects {0, 1}, {0, 2}, {1, 0}, {1, 1}, the four of the range. They are what moqtail's test client wrote as the publisher. Its Subgroup IDs are 0 for Group 0 and 1 for Group 1 (written as "the prior one plus one"), where the moqtail relay's answer to the same FETCH has Subgroup 1 throughout (`fetch-stream-from-moqtail-relay`); the Objects and payloads are the same. Run 3 was answered with the End Location that was asked for: the publisher had sent Objects beyond Group 2 by then, so the relay knew a larger Location.

**The relay core treats what has no FETCH as the revision before did.** `spikes/relay-core-diff`, `7f264197` (the revision before the relay served a FETCH) against the working copy, 20,000 random sequences of 80 events in each of five worlds:

| World | Sequences in which the versions differ | Bystanders treated differently | Events of a kind the old core does not know |
|---|---|---|---|
| `full` | 0 | not counted in this world | 0 |
| `one-publisher` | 0 | not counted in this world | 0 |
| `split-one` | 0 | 0 | 0 |
| `shared` | 0 | 0 | 0 |
| `no-announce` | 0 | not counted in this world | 0 |

## Limitations and blockers

No blocker.

- **One run each, with one publisher.** The bytes of the fetch stream were not compared with what the publisher wrote, which nothing here records; that the relay passes a stream on unchanged is held by a test on in-memory connections (300,000 bytes that are no Objects).
- **Of what cuts a FETCH short, two were run against a real peer** (runs 8 and 9): a subscriber that takes nothing, and a publisher that leaves. A subscriber that cancels, a publisher that sends nothing, one that resets its stream and one that answers without opening a stream are tests on in-memory connections with the clock stopped. That a fetch stream nobody reads holds its sender back, and that a fetch stream whose writer is dropped is reset and does not end with a FIN, are also tested over a real loopback connection, in `xmoq-moqt-io`.
- `moqlivemock`'s publisher was not sent a FETCH with FILL_TIMEOUT or GROUP_ORDER. A publisher that does not know a parameter closes its session (§10.2.1), which every subscriber of it shares.
- **The comparison of the relay core says nothing about a FETCH.** No world of the tool has one. It says that sequences without a FETCH lead to the same actions as before. Its tests of the new behaviour differ between the versions, as they are meant to, and are not counted.
- A Joining FETCH is refused (run 5), so a viewer that subscribes with one, as `moqlivemock`'s does for the catalog, gets no catalog through the relay yet.
- The moqtail npm package, `moqlivemock` and a browser were not run.
- **The comparison of the relay core has `7f264197` as its "before", and not the revision of this change before its two reviews** (`538bf101`). Against that one the tool ends with `NOT COMPARED`: the two cores are built with one set of actions, the review added fields to an action of FETCH, and the older core cannot make it. What the comparison with `7f264197` says is said of all of the change.
- Run 9 was meant to be run 8: the publisher was done sooner than expected and left. It is kept as the run of a publisher that leaves.
