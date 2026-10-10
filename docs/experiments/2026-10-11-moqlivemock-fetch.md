# FETCH bytes from moqlivemock and its library

- Date: 2026-10-11
- Commit SHA: the commit that adds this record. The own decoders that read the bytes are those of the change that adds FETCH to both codecs (`crates/moqt/src/fetch.rs` and `packages/moqt/src/core/fetch.ts`, on `bad83ca4` and the TypeScript commit on top of it)
- Roadmap item: groundwork for Phase 1 (FETCH). It closes no item

## Objective

The fixtures for FETCH were taken from the moqtail relay, which writes no End of Range indicator and no Descending response, and reads two places of the draft another way than the own implementations. Take the same from a second implementation, moqlivemock v0.16.1 and the library under it, and see which way it reads those places:

- whether the two fields of an End of Range indicator (Serialization Flags `0x8C`, `0x10C`) are a Location or deltas;
- what "the prior Object" is for the Object after an indicator, also when no Object came before the indicator;
- whether a range that ends at `{G, 0}` includes Group G, and from where a Joining FETCH's range is counted.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4 (23E214), arm64, loopback, ports 47200 to 47240 |
| moqlivemock | `Eyevinn/moqlivemock` `v0.16.1` (`91d903e9933a452f8baaa860f9fcd20cf195e077`), in a copy outside the repository, built with Go 1.26.0 |
| Its library | `Eyevinn/moqtransport` v0.14.0, which encodes and decodes every MOQT byte of moqlivemock |
| Changes to the copy | [`spikes/moqlivemock-fetch/`](../../spikes/moqlivemock-fetch/): a layer around the library's connection that prints the bytes of every stream as they are read and written (`MLM_TEE`), and a command `fetchcap` |
| Transport | raw QUIC, ALPN `moqt-18`, between moqlivemock's own parts. No own client or relay took part |

**How far each capture is moqlivemock's.** Three kinds, and the record keeps them apart:

| Kind | Who decided what was sent | Who encoded it |
|---|---|---|
| `mlmsub` against `mlmpub`, unchanged but for the tee | moqlivemock | its library |
| `fetchcap fetch` against `mlmpub` and `mlmrel` | the requests: this spike (a Standalone FETCH with a range). The answers: moqlivemock | its library, both |
| `fetchcap fetch` against `fetchcap serve` | this spike: the publisher writes a scripted list of Objects and indicators, with the values of the hand-written files of `fixtures/moqt-18-by-hand/` | its library, both. **No moqlivemock command writes an End of Range indicator itself**: `mlmpub` answers a FETCH for its catalog only, with one Object, and `mlmrel` serves a range its cache covers whole or passes on what upstream wrote (read in `internal/pub/pub.go` and `internal/relay/fetch.go`) |

## Commands

```sh
# in the copy of moqlivemock, with spikes/moqlivemock-fetch/ applied
go build -o ../bin/mlmpub-tee ./cmd/mlmpub      # and mlmsub, mlmrel, fetchcap
(cd cmd/mlmpub && MLM_TEE=1 mlmpub-tee -addr 127.0.0.1:47200)
MLM_TEE=1 mlmsub-tee -addr 127.0.0.1:47200 -namespace mlm/msf/clear -duration 3 -videoout /dev/null
MLM_TEE=1 fetchcap fetch 127.0.0.1:47200 mlm/msf/clear catalog none 0 0 0 0         # Start {0,0}, End {0,0}
mlmrel-tee -addr 127.0.0.1:47220 -upstream moqt://127.0.0.1:47200                   # and an mlmsub through it
MLM_TEE=1 fetchcap fetch 127.0.0.1:47220 mlm/msf/clear video_400kbps_avc descending <G> 0 <G+1> 0
MLM_TEE=1 fetchcap serve 127.0.0.1:47240
MLM_TEE=1 fetchcap fetch 127.0.0.1:47240 x/y end-of-range none 0 0 10 0             # and the four other scripts
# the captured bytes, a byte at a time, through both own decoders
cargo run --offline -- out/scripted-streams.txt      # spikes/moqlivemock-fetch/decode-rs.rs
bun decode-ts.ts out/scripted-streams.txt
```

## Observed results

### 1. `mlmsub` and `mlmpub`

| | Bytes |
|---|---|
| SUBSCRIBE for the catalog, and its SUBSCRIBE_OK with LARGEST_OBJECT {0, 0} | `03 00 19 00 03 03 6d 6c 6d 03 6d 73 66 05 63 6c 65 61 72 07 63 61 74 61 6c 6f 67 00`; `04 00 05 00 01 09 00 00` |
| FETCH: Request ID 2, Relative Joining, Joining Request ID 0, Joining Start 0, no parameters | `16 00 05 02 02 00 00 00` |
| FETCH_OK: End Of Track 0, End Location {0, 1} | `18 00 04 00 00 01 00` |
| The fetch stream: FETCH_HEADER with Request ID 2, then one Object | `05 02`, then `1c 00 00 00 8a 9b` and 2,715 bytes of JSON, then a FIN |

- The Object is the catalog, {0, 0}, flags `0x1C`: Group ID, Object ID and priority present, Subgroup ID 0 by the mode. moqtail writes the Subgroup ID of a first Object as a field (`0x1F`); both are within the draft.
- The same Object came on a subgroup stream of the subscription as well, as recorded before.
- `mlmpub` logged the range it resolved: Start {0, 0}, End {0, 1}, which is the LARGEST_OBJECT it had given in SUBSCRIBE_OK plus one Object. **Whether it counts from the SUBSCRIBE_OK or from the moment the FETCH arrives cannot be told here**: its catalog is one Object that never moves.

### 2. The End of Range indicator, in the library's source and on the wire

Read in `Eyevinn/moqtransport` v0.14.0, `internal/wire2/fetch_object.go`:

- Lines 284 to 306 (`readEndOfRange`): the two fields are read as two integers and are the Group ID and Object ID as they stand. Lines 302 and 303 make them the prior Location; "Only the Location advances: Subgroup ID and Priority still refer to the last real Object".
- Lines 122 to 125: "Group ID and Object ID advance past an End of Range indicator, while Subgroup ID and Priority carry over from the last *actual* Object".
- Lines 171 to 173: the first record needs both deltas when there is no prior Location; **after an indicator there is one**, so an Object that follows an indicator alone may leave them out or give deltas.
- Lines 310 to 321 and `object.go` 100 to 105: a Group ID Delta is `prior + delta + 1` ascending and `prior - delta - 1` descending.

**That is the reading of the own implementations in all three places, and not moqtail's in the second** (moqtail takes the Object before the indicator as the prior one).

The scripted publisher wrote, through that library:

| Script | Group Order | Stream |
|---|---|---|
| `end-of-range` | Ascending | `05 00` `1c 00 00 07 01 aa` `80 8c 00 05` `14 01 07 01 bb` `81 0c 02 00` `14 03 07 01 cc` |
| `indicator-first` | Ascending | `05 00` `81 0c 03 01` `14 01 07 00` |
| `indicator-first-group-delta` | Ascending | `05 00` `81 0c 03 01` `1c 05 00 07 00` |
| `subgroup-id-modes` | Ascending | `05 00` `5c 01 00 07 00` `1f 00 04 00 09 00` `17 05 01 09 00` |
| `descending` | Descending | `05 00` `1c 09 00 07 01 aa` `1c 01 01 07 01 bb` `1c 00 05 07 01 cc` |

- `80 8c 00 05`: the indicator carries the Location {0, 5} as it stands, and nothing after it. The Object behind it, `14 01 …`, has Object ID Delta 1 and no Group: {0, 5 + 1}.
- `81 0c 03 01` then `14 01 07 00`: an indicator with no Object before it, and an Object that counts from it: {3, 2}.
- `81 0c 03 01` then `1c 05 00 07 00`: Group ID Delta 5 from the indicator's Group 3 is Group 9. **These eleven bytes are the hand-written file `fetch-stream-indicator-first-group-delta` byte for byte.**
- The other four differ from the hand-written files in which fields are left out, and decode to the same values: this library writes the priority with every Object (`0x14` where the hand-written file has `0x00`, `0x1c` where it has `0x0c`) and writes a Subgroup ID that is the prior one plus one as a field (`0x17`).

**Both own decoders read all five as the library's own reader did**, a byte at a time:

| Script | Decoded, by the Rust and by the TypeScript decoder |
|---|---|
| `end-of-range` | header 0; Object {0, 0}; End of Non-Existent Range {0, 5}; Object {0, 6}; End of Unknown Range {2, 0}; Object {2, 3}; Subgroup 0, priority 7 throughout |
| `indicator-first` | header 0; End of Unknown Range {3, 1}; Object {3, 2}, Subgroup 0, priority 7 |
| `indicator-first-group-delta` | header 0; End of Unknown Range {3, 1}; Object {9, 0}, Subgroup 0, priority 7 |
| `subgroup-id-modes` | header 0; Object {1, 0} without a Subgroup ID, priority 7; Object {2, 0} in Subgroup 4, priority 9; Object {2, 1} in Subgroup 5 |
| `descending` | header 0; Objects {9, 0}, {7, 1}, {6, 5}, Subgroup 0, priority 7 |

### 3. Ranges, and a Descending response of real media

Standalone FETCHes of `fetchcap` against `mlmpub`, for its catalog, whose one Object is {0, 0}:

| Start | End | Answer |
|---|---|---|
| {0, 0} | {0, 0} | FETCH_OK with End Location {0, 0}; the stream has the Object |
| {0, 0} | {1, 0} | End Location {1, 0}; the Object |
| {0, 0} | {0, 1} | End Location {0, 1}; the Object |
| {0, 1} | {1, 0} | End Location {1, 0}; `05 00` and a FIN, no Object |
| {0, 0} | {1, 0}, GROUP_ORDER Descending | End Location {1, 0}; the Object |

- **End {0, 0} is the whole of Group 0** here ("A Location.Object value of 0 means the entire group is requested"), where the moqtail relay leaves Group G out for End {G, 0}.
- FETCH_OK repeats the End of the request, End Of Track 0, no parameters, no Track Properties.
- A FETCH for a video track is refused: REQUEST_ERROR `0x10` (DOES_NOT_EXIST), Retry Interval 0, "only catalog is fetchable".

Against `mlmrel` with `mlmpub` upstream, while an `mlmsub` through the relay kept the video track in its cache (Group IDs are the wall-clock second, 25 Objects a Group):

| Request | Answer |
|---|---|
| Start {G, 0}, End {G+1, 0} | 50 Objects: Group G, then Group G+1, each from Object 0 to 24. So End {G+1, 0} is the whole of Group G+1 here as well |
| The same, GROUP_ORDER Descending | 50 Objects: **Group G+1 first, then Group G, the Objects of each from 0 to 24**. 92,881 bytes |
| Start {G, 0}, End {G, 0} | refused, `0x10`, "only catalog is fetchable": the relay did not serve it from its cache and passed it upstream. Why its cache did not count one Group as covered was not read |

- Every Object of these streams has the property `0x10` (the timestamp) and a payload of 900 to 19,000 bytes; the first is `3c f0 6a ca 9a dd 00 80 09 10 fe …`, flags `0x3C`.
- **The Rust and the TypeScript decoder read both streams to the same 51 items**, compared as text: the header and 50 Objects with the same Group, Object, Subgroup 0, priority 128 and payload lengths.

### 4. Control messages

Nine captured messages were decoded by both own decoders to the same values, and encoded again by the TypeScript one to the same bytes: the Relative Joining FETCH and its FETCH_OK, three Standalone FETCHes (one with GROUP_ORDER Descending, one with Group IDs of five bytes), three FETCH_OKs, and the REQUEST_ERROR. They are in [`spikes/moqlivemock-fetch/out/messages.txt`](../../spikes/moqlivemock-fetch/out/messages.txt).

### 5. A FETCH with FILL_TIMEOUT, and one with GROUP_ORDER, sent to `mlmpub` and to `mlmrel`

Added later on 2026-10-11, with the own relay about to copy a subscriber's GROUP_ORDER and FILL_TIMEOUT into the FETCH it sends a publisher. §10.2.1 has a receiver close the session for a parameter it does not know in a message, so one subscriber's FETCH could end the session every subscriber of that publisher uses. The question: does this publisher close the session?

Three Standalone FETCHes for (`mlm`, `msf`, `clear`) / `catalog`, {0, 0} to {1, 0}, each in a session of its own, written by hand and sent with `spikes/fetch-capture` (it writes the bytes it is given and prints what comes back; SETUP `af 00 00 00`). **Both hand-written messages were decoded by the own TypeScript decoder before they were sent**, and read as meant: FILL_TIMEOUT 500, and GROUP_ORDER Descending.

```sh
N=03036d6c6d036d736605636c65617207636174616c6f67      # the Track Namespace and Track Name
C="spikes/fetch-capture/target/debug/xmoq-fetch-capture --url https://127.0.0.1:47200/moq --cert-sha256 <fingerprint mlmpub prints> --control af000000 --wait-ms 3000"
$C --request "16001e00 01 $N 0000 0100 00"              # no parameter
$C --request "16002100 01 $N 0000 0100 01 0a 81f4"      # FILL_TIMEOUT (0x0A) 500: 81 f4 by §1.4.1
$C --request "16002000 01 $N 0000 0100 01 22 02"        # GROUP_ORDER (0x22) Descending
# and the same three to mlmrel on 47220, started with -upstream moqt://127.0.0.1:47200
```

| Sent to | FETCH | Answer | The session |
|---|---|---|---|
| `mlmpub` | no parameter | FETCH_OK `18 00 04 00 01 00 00` (End Location {1, 0}), and a fetch stream with the catalog as Object {0, 0}, ended with a FIN | open until the tool left, 3 seconds later |
| `mlmpub` | FILL_TIMEOUT 500 | the same | the same |
| `mlmpub` | GROUP_ORDER Descending | the same | the same |
| `mlmrel` | all three | the same three answers, from `mlmpub`: its log has "proxying fetch" | open, and so was the session between `mlmrel` and `mlmpub`, which served all three |

- **Neither closed a session, and neither refused.** `mlmpub` logged "served catalog via FETCH" for each. Its session ended when the tool left without closing in order ("peer closed its control stream"), 3 seconds after the FETCH_OK.
- **`mlmrel` does not pass FILL_TIMEOUT on, and always sends a GROUP_ORDER.** What `mlmpub` received from it for the three: `... 01 22 01`, `... 01 22 01` and `... 01 22 02`, with Request IDs 2, 4 and 6. So the FETCH with FILL_TIMEOUT went upstream without it and with GROUP_ORDER Ascending, and the one with Descending kept its order.
- Whether either of them does anything with the two parameters was not seen: the catalog is one Object in one Group, so no order shows, and nothing has to be waited for.
- One run each. `mlmpub` answers a FETCH for its catalog only, so no other track was asked for.

## Limitations and blockers

- **The indicators were written by the library because this spike told it to**, with values chosen here. They show how that library encodes and decodes an indicator, which is what the readings in question are about; they are not something a deployed publisher sent.
- Nothing ran against an own client or relay: the own implementations do not send or serve a FETCH yet. The captured bytes were read by the own decoders in two small programs.
- The Joining FETCH's range (the first of moqtail's deviations) could not be told apart from a range counted at the time of the FETCH.
- No Absolute Joining FETCH, no FETCH with FILL_TIMEOUT or a token, no fetch stream that is reset, and no Object with the Datagram bit from a moqlivemock command (the scripted one has it).
- The files are not in `fixtures/` yet. The two video streams are not kept at all: they are moqlivemock's test asset.
- WebTransport was not used; the same library writes the same MOQT bytes over it.

## Open questions raised

- Which of the captured streams become fixtures. `indicator-first-group-delta` is byte for byte the hand-written file, which could then stand as a capture; the other four decode to the hand-written values from other bytes.
- Whether the second deviation of moqtail (End {G, 0}) matters to the own relay's FETCH: two implementations now answer the same request with different Objects.
