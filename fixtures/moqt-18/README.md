# MOQT draft-18 wire fixtures

Bytes that moqtail put on the wire, used as fixed expectations by XmoQ's own implementations. A codec that only decodes what it encoded itself cannot notice that both directions misread the draft in the same way; these files come from another implementation.

## Format

One file per control message or per stream. The content is hexadecimal digits; spaces and newlines are ignored. A control message file holds the whole message, including Message Type and Message Length. A subgroup stream file holds the stream from its first byte: the Subgroup Header, then the Objects.

Tests read these files and are expected to do both: decode the bytes into the value listed below, and encode that value back into the same bytes. The Rust tests (`crates/moqt`) and the TypeScript tests (`packages/moqt/test/core/moqtail-fixtures.test.ts`) read every file.

## Source

All bytes were captured on 2026-10-10 on macOS 14.4 arm64 over loopback, against the moqtail relay 0.15.0 (`moqtail/0.15.0+dd8623d`, tag `relay@0.15.0`). Namespace `xmoq/phase0`, track `object`.

Two ways of capturing were used.

- **Relay log.** The relay prints a hex dump for each control message, such as `Subscribe (34 bytes): 03 00 1f ...`. These are the bytes on the wire, not a re-encoding: for a received message the relay prints the slice of its receive buffer that the message occupied, and for a sent message it prints the buffer it writes to the stream next. Checked on 2026-10-10 in `libs/moqtail-rs/src/transport/control_stream_handler.rs` of `moqtail/moqtail` at tag `relay@0.15.0` (`send`, lines 79–95, and `next_message`, lines 165–186).
- **Client trace.** `xmoq-moqt-client --trace` prints the bytes of every chunk as it comes off a stream.
- **Page trace.** The own page of `interop/` prints the bytes of every chunk as it comes off a stream, with `--verbose` (capture H, on 2026-10-11).

| File | Sender | Captured from | How |
|---|---|---|---|
| `setup-moqtail-ts.hex` | moqtail npm 0.13.0 in Chromium 156 | relay log | capture A |
| `setup-moqtail-relay.hex` | relay | relay log | capture A |
| `publish-namespace-moqtail-ts.hex` | moqtail npm 0.13.0 | relay log | capture A |
| `request-ok-publish-namespace-moqtail-relay.hex` | relay, answering PUBLISH_NAMESPACE | relay log | capture A |
| `publish-moqtail-ts.hex` | moqtail npm 0.13.0 | relay log | capture A |
| `request-ok-publish-moqtail-relay.hex` | relay, answering PUBLISH | relay log | capture A |
| `subscribe-moqtail-rs.hex` | `moqtail` crate 0.15.0 (spike client) | relay log | capture A |
| `subscribe-ok-moqtail-relay.hex` | relay | relay log | capture A |
| `publish-done-moqtail-relay.hex` | relay, after the publisher left | relay log | capture A |
| `subscribe-moqtail-ts.hex` | moqtail npm 0.13.0 | relay log | capture B |
| `publish-moqtail-rs.hex` | `moqtail` crate 0.15.0 (spike client) | relay log | capture C |
| `subgroup-stream-from-moqtail-ts.hex` | relay, forwarding an Object published by moqtail npm 0.13.0 | client trace | capture D |
| `subgroup-stream-from-moqtail-rs.hex` | relay, forwarding an Object published by the `moqtail` crate 0.15.0 (spike client) | client trace | capture E |
| `request-error-does-not-exist-moqtail-relay.hex` | relay, answering a SUBSCRIBE for a track nobody published | client trace | capture F |
| `subscribe-ok-largest-object-moqtail-relay.hex` | relay, answering a SUBSCRIBE for a track that already had Objects | client trace | capture G |
| `subgroup-stream-object-id-gaps-from-moqtail-rs.hex` | relay, forwarding Objects published by moqtail's own test client 0.15.0 | client trace | capture G |
| `subgroup-stream-joined-mid-group-from-moqtail-rs.hex` | same | client trace | capture G |
| `subgroup-stream-object-properties-from-own-ts.hex` | relay, forwarding an Object with two Object Properties published by XmoQ's own browser client (`@xmoq/moqt` at `a2a3884a`) in Chromium 156 | page trace | capture H |
| `fetch-standalone-moqtail-rs.hex` | moqtail's test client 0.15.0 | its log | capture I |
| `fetch-ok-moqtail-relay.hex` | relay, answering that FETCH | log of the test client | capture I |
| `fetch-relative-joining-moqtail-rs.hex` | moqtail's test client 0.15.0, after a SUBSCRIBE | its log | capture J |
| `fetch-ok-joining-moqtail-relay.hex` | relay, answering that FETCH | log of the test client | capture J |
| `fetch-stream-from-moqtail-relay.hex` | relay, answering the FETCH of capture I sent again | capture tool | capture K |
| `fetch-stream-five-groups-from-moqtail-relay.hex` | relay, answering an Absolute Joining FETCH written by hand | capture tool | capture L |
| `fetch-stream-gaps-from-moqtail-relay.hex` | relay, answering a FETCH written by hand for a track with gaps in its Object IDs and Group IDs | capture tool | capture M |
| `fetch-stream-empty-from-moqtail-relay.hex` | relay, answering a FETCH written by hand whose range it takes to hold nothing | capture tool | capture N |

Captures A to E use `spikes/moqtail/browser/run.mjs` from the repository root:

```sh
node spikes/moqtail/browser/run.mjs --direction browser-to-rust                                                      # A
node spikes/moqtail/browser/run.mjs --direction browser-to-browser                                                   # B
node spikes/moqtail/browser/run.mjs --direction rust-to-browser                                                      # C
node spikes/moqtail/browser/run.mjs --direction browser-to-rust --rust-subscriber own --trace                        # D
node spikes/moqtail/browser/run.mjs --direction rust-to-rust --rust-publisher moqtail --rust-subscriber own --trace  # E
```

Captures F and G start the relay by hand. `<sha256>` is the certificate hash that `run.mjs` prints as `certificate sha256=`.

```sh
# The relay, left running in another terminal for F and G.
spikes/moqtail/.tools/bin/relay --port 44361 --host 127.0.0.1 \
  --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem \
  --log-folder spikes/moqtail/.logs

# F: nobody has published the track.
target/debug/xmoq-moqt-client --mode subscribe --url https://127.0.0.1:44361 \
  --cert-sha256 <sha256> --namespace xmoq/phase0 --track object --trace

# G: moqtail's test client publishes Object IDs 0, 2 and 4 in each Group, in another terminal.
cargo install --git https://github.com/moqtail/moqtail --tag relay@0.15.0 --locked \
  --root spikes/moqtail/.tools client
spikes/moqtail/.tools/bin/client --command publish --server moqt://127.0.0.1:44361 \
  --no-cert-validation --namespace xmoq/phase0 --track-name object \
  --group-count 8 --objects-per-group 3 --object-id-step 2 --interval 400 --payload-size 8

# G: subscribe while it is publishing.
target/debug/xmoq-moqt-client --mode subscribe --objects 7 --url https://127.0.0.1:44361 \
  --cert-sha256 <sha256> --namespace xmoq/phase0 --track object --trace
```

Capture H, from `interop/`; the file is the line `bytes received peer-uni-1` of the subscriber:

```sh
bun run.ts --publisher own-browser --subscriber own-browser --relay moqtail --object-properties --verbose  # H
```

Captures I to N are of FETCH. The relay and moqtail's test client are started by hand as for G; the test client logs each control message it sends and receives in the form of the relay log.

```sh
# I: while the test client publishes 12 Groups of 3 Objects, 250 ms apart.
spikes/moqtail/.tools/bin/client --command fetch --server moqt://127.0.0.1:48600 \
  --no-cert-validation --namespace xmoq/phase0 --track-name object \
  --start-group 0 --start-object 1 --end-group 1 --end-object 2

# J: a SUBSCRIBE followed by a Relative Joining FETCH.
spikes/moqtail/.tools/bin/client --command subscribe --server moqt://127.0.0.1:48600 \
  --no-cert-validation --namespace xmoq/phase0 --track-name object \
  --joining-fetch --joining-start 0 --duration 2
```

The test client does not log the fetch stream. For K to N the bytes are sent and read by `spikes/fetch-capture`, which writes what it is given in hexadecimal on a control stream and on one request stream each, and prints what comes back on every stream; it encodes and decodes nothing.

```sh
cargo build --manifest-path spikes/fetch-capture/Cargo.toml
C="spikes/fetch-capture/target/debug/xmoq-fetch-capture --url https://127.0.0.1:48600 --cert-sha256 <sha256> --control af000000"
N="02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74"   # (xmoq, phase0), object

# K: the FETCH of capture I, as the test client sent it.
$C --request "$(cat fixtures/moqt-18/fetch-standalone-moqtail-rs.hex)"
# L: the SUBSCRIBE of `subscribe-moqtail-rs`, then an Absolute Joining FETCH from Group 2.
$C --request "$(cat fixtures/moqt-18/subscribe-moqtail-rs.hex)" --request "16 00 07 02 03 00 02 01 20 c8"
# M: the publisher with --objects-per-group 2 --object-id-step 2 --group-id-step 2; Start {0, 0}, End {4, 0}.
$C --request "16 00 1b 00 01 $N 00 00 04 00 00"
# N: Start {1, 0}, End {1, 0}.
$C --request "16 00 1b 00 01 $N 01 00 01 00 00"
```

**The requests of L, M and N were written by hand**, from the two FETCH messages of captures I and J with fields changed. They are no fixtures. A test of `crates/moqt` (`the_fetches_sent_to_take_the_fixtures_say_what_they_were_meant_to`) decodes each of them with `ControlMessage::decode` and compares it with the request that was meant: an Absolute Joining FETCH with Request ID 2, Joining Request ID 0, Joining Start 2 and SUBSCRIBER_PRIORITY 200; and Standalone FETCHes for {0, 0} to {4, 0} and for {1, 0} to {1, 0}. One request of this kind was first written with the variable-length integers of QUIC, which are not those of §1.4.1, and the relay answered another request than was meant; that is why they are read.

**What came back differs from draft 18 in places** ([record](../../docs/experiments/2026-10-11-moqtail-relay-fetch.md)); for some of them it is not known whether the relay decides or the publisher behind it, moqtail's test client, to which the relay passes a FETCH it cannot answer itself. The files are fixtures for how the bytes are laid out, and not for which Objects a FETCH is to return.

**The file of capture H is moqtail's encoding of a value an own client chose.** No moqtail client in this repository sends an Object Property with content, so the two properties came from the own browser client. The relay does not pass the bytes through: it parses each Object into a list of properties and serializes it again with moqtail's encoder, the type of each property as a delta from the one before. Read on 2026-10-11 at `dd8623d` in `apps/relay/src/server/subscription.rs` (`object.try_into_subgroup()`, then `sub_object.serialize(previous_object_id, has_properties)`) and `libs/moqtail-rs/src/model/common/pair.rs` (`serialize_delta`). So the file shows that moqtail's parser took what the own encoder wrote and that moqtail's encoder writes the same bytes for it; it does not show a property moqtail made up itself.

Some values depend on the run and will differ when a capture is repeated: the Track Alias moqtail-ts picks at random (779698128977928661 in `publish-moqtail-ts`), the timestamp and nonce in the two 53-byte payloads, the 8 random payload bytes of capture G, and the Group that a subscriber happens to join in capture G. A repeated capture is a new fixture, not a check of the old one.

The subgroup streams are what the relay wrote, not what the publisher wrote: the relay re-serializes the Subgroup Header with the subscriber's Track Alias. The streams of captures D and E had not been closed when they were captured, so those files end after the first Object. The files of capture G hold what had arrived when the subscriber left; whether a FIN followed was not observed.

## Decoded values

| File | Value |
|---|---|
| `setup-moqtail-ts` | SETUP, no Setup Options |
| `setup-moqtail-relay` | SETUP, MOQT_IMPLEMENTATION (0x07) = `moqtail/0.15.0+dd8623d` |
| `publish-namespace-moqtail-ts` | PUBLISH_NAMESPACE, Request ID 0, Track Namespace (`xmoq`, `phase0`), no parameters |
| `request-ok-publish-namespace-moqtail-relay` | REQUEST_OK, no parameters, no Track Properties |
| `publish-moqtail-ts` | PUBLISH, Request ID 2, (`xmoq`, `phase0`) / `object`, Track Alias 779698128977928661, FORWARD = 1, no Track Properties |
| `publish-moqtail-rs` | PUBLISH, Request ID 0, (`xmoq`, `phase0`) / `object`, Track Alias 1, FORWARD = 1, no Track Properties |
| `request-ok-publish-moqtail-relay` | REQUEST_OK, FORWARD = 1, GROUP_ORDER = Ascending, no Track Properties |
| `subscribe-moqtail-rs` | SUBSCRIBE, Request ID 0, (`xmoq`, `phase0`) / `object`, FORWARD = 1, SUBSCRIBER_PRIORITY = 128, SUBSCRIPTION_FILTER = Largest Object, GROUP_ORDER = Ascending |
| `subscribe-moqtail-ts` | SUBSCRIBE, Request ID 0, (`xmoq`, `phase0`) / `object`, FORWARD = 1, SUBSCRIBER_PRIORITY = 128, SUBSCRIPTION_FILTER = Largest Object |
| `subscribe-ok-moqtail-relay` | SUBSCRIBE_OK, Track Alias 0, no parameters, no Track Properties |
| `subscribe-ok-largest-object-moqtail-relay` | SUBSCRIBE_OK, Track Alias 0, LARGEST_OBJECT = {Group 3, Object 2}, no Track Properties |
| `publish-done-moqtail-relay` | PUBLISH_DONE, Status Code 2 (TRACK_ENDED), Stream Count 1, Error Reason `Publisher disconnected` |
| `request-error-does-not-exist-moqtail-relay` | REQUEST_ERROR, Error Code 0x10 (DOES_NOT_EXIST), Retry Interval 0, Error Reason `Unknown track namespace` |
| `subgroup-stream-from-moqtail-ts` | SUBGROUP_HEADER type 0x58 (Subgroup ID 0 by mode, END_OF_GROUP, FIRST_OBJECT), Track Alias 0, Group ID 0, Publisher Priority 0. One Object: Object ID 0, payload `xmoq-phase0 2026-10-10T01:18:43.029Z f69e3bea2a578c6b` |
| `subgroup-stream-from-moqtail-rs` | SUBGROUP_HEADER type 0x5D (PROPERTIES, explicit Subgroup ID, END_OF_GROUP, FIRST_OBJECT), Track Alias 0, Group ID 0, Subgroup ID 0, Publisher Priority 0. One Object: Object ID 0, Properties Length 0, payload `xmoq-phase0 2026-10-10T01:19:03.764Z bb30f5dd09c8981f` |
| `subgroup-stream-object-id-gaps-from-moqtail-rs` | SUBGROUP_HEADER type 0x5D, Track Alias 0, Group ID 4, Subgroup ID 1, Publisher Priority 128. Three Objects with the payload `4d 3b c3 24 4c 84 57 2d`: Object ID 0 with no properties; Object ID 2 (Object ID Delta 1) and Object ID 4 (Object ID Delta 1), each with the property 0x3E (Prior Object ID Gap) = 1 |
| `subgroup-stream-joined-mid-group-from-moqtail-rs` | SUBGROUP_HEADER type 0x5D, Track Alias 0, Group ID 3, Subgroup ID 1, Publisher Priority 128. One Object: Object ID 4 (Object ID Delta 4 as the first on the stream), property 0x3E = 1, payload `4d 3b c3 24 4c 84 57 2d`. Type 0x5D has FIRST_OBJECT set although Object ID 4 is not the first Object published in the Subgroup; see [the experiment record](../../docs/experiments/2026-10-10-own-rust-client-vs-moqtail.md) |
| `fetch-standalone-moqtail-rs` | FETCH, Request ID 0, Fetch Type 1 (Standalone), (`xmoq`, `phase0`) / `object`, Start Location {0, 1}, End Location {1, 2}, SUBSCRIBER_PRIORITY = 200 |
| `fetch-ok-moqtail-relay` | FETCH_OK, End Of Track 0, End Location {1, 2}, no parameters, no Track Properties |
| `fetch-relative-joining-moqtail-rs` | FETCH, Request ID 2, Fetch Type 2 (Relative Joining), Joining Request ID 0, Joining Start 0, SUBSCRIBER_PRIORITY = 200 |
| `fetch-ok-joining-moqtail-relay` | FETCH_OK, End Of Track 0, End Location {6, 2}, no parameters, no Track Properties. The SUBSCRIBE_OK before it had LARGEST_OBJECT = {6, 1} |
| `fetch-stream-from-moqtail-relay` | FETCH_HEADER, Request ID 0. Four Objects with the payload `4d 3b c3 24 4c 84 57 2d`, Subgroup 1, Publisher Priority 128: {0, 1} with Serialization Flags 0x1F (Group ID, Subgroup ID, Object ID and priority present), {0, 2} with 0x01 (the prior Subgroup; Object ID the prior one plus one), {1, 0} with 0x0D (Group ID Delta 0, Object ID 0), {1, 1} with 0x01 |
| `fetch-stream-five-groups-from-moqtail-relay` | FETCH_HEADER, Request ID 2. Thirteen Objects of the same kind: Objects 0 to 2 of Groups 2 to 5, and {6, 0}. Flags 0x1F for the first, 0x0D for the first of each further Group, 0x01 for the others. The SUBSCRIBE_OK of that run had LARGEST_OBJECT = {5, 2} (`04 00 05 00 01 09 05 02`) and the FETCH_OK the End Location {6, 1} (`18 00 04 00 06 01 00`): {6, 0} lies after the Joining Location, where draft 18 ends a Joining FETCH (divergence a of the record) |
| `fetch-stream-gaps-from-moqtail-relay` | FETCH_HEADER, Request ID 0. Four Objects of the same kind: {0, 0} with flags 0x1F; {0, 2} with 0x25 (Object ID Delta 2, Properties present), property 0x3E (Prior Object ID Gap) = 1; {2, 0} with 0x2D (Group ID Delta 1, Object ID 0, Properties present), property 0x3C (Prior Group ID Gap) = 1; {2, 2} with 0x25, property 0x3E = 1. The End Location asked for was {4, 0} and nothing of Group 4 is there |
| `fetch-stream-empty-from-moqtail-relay` | FETCH_HEADER, Request ID 0, and a FIN |
| `subgroup-stream-object-properties-from-own-ts` | SUBGROUP_HEADER type 0x5D, Track Alias 0, Group ID 0, Subgroup ID 0, Publisher Priority 0. One Object: Object ID 0, Properties Length 29, two properties, payload `xmoq-phase0b 2026-10-10T15:06:02.523Z c5e55f133a0eddce` (54 bytes). Property 0x0D, an odd type: Delta Type 13, Length 18, the bytes `01 42 e0 1f ff e1 00 09 67 42 e0 1f 8c 8d 40 50 00 00`. Property 0x10, an even type: Delta Type 3, the integer 1760140800123456 in 8 bytes (`fe 06 40 d6 b7 25 62 40`). Neither type is a property draft 18 defines; they are the numbers draft-ietf-moq-loc-04 gives Video Config and Timestamp, and the values are made up |

## Encodings not checked against moqtail

No fixture here covers the following. XmoQ's tests for them use bytes written by hand from the draft, so the reader of the draft and the writer of the expectation are the same.

- REQUEST_UPDATE (0x02).
- Message Parameters OBJECT_DELIVERY_TIMEOUT (0x02), AUTHORIZATION_TOKEN (0x03), RENDEZVOUS_TIMEOUT (0x04), SUBGROUP_DELIVERY_TIMEOUT (0x06), EXPIRES (0x08) and NEW_GROUP_REQUEST (0x32).
- SUBSCRIPTION_FILTER with Next Group Start (0x1), AbsoluteStart (0x3) or AbsoluteRange (0x4), and GROUP_ORDER = Descending.
- REQUEST_ERROR with a Redirect.
- Track Properties in any message, and Setup Options other than MOQT_IMPLEMENTATION.
- Subgroup Headers with SUBGROUP_ID_MODE 0b01 or with DEFAULT_PRIORITY set.
- The Object Status field: End of Group, End of Track, and a Normal Object with an empty payload.
- An Object Property with a byte value (an odd type).
- Padding streams, and varints in a longer encoding than needed.
- An Absolute Joining FETCH (Fetch Type 0x3) as a message, and a FETCH with FILL_TIMEOUT (0x0A), GROUP_ORDER or AUTHORIZATION_TOKEN. FETCH_OK with End Of Track 1 or with Track Properties.
- On a fetch stream: the End of Range indicators (0x8C, 0x10C), a stream in Descending Group Order, an Object with the Datagram bit (0x40), Subgroup ID mode 0x00 and 0x02, and an Object ID that is left out for the first Object of a Group. The moqtail relay 0.15.0 did not send a stream in Descending order when asked (the record above).

For RENDEZVOUS_TIMEOUT the draft does not state the value encoding (§10.2.6). XmoQ encodes it as a varint. The `moqtail` crate 0.15.0 does the same: `serialize_message_parameters` in `libs/moqtail-rs/src/model/parameter/message_parameter.rs` of `moqtail/moqtail` at tag `relay@0.15.0` writes its value with `put_vi`, the varint writer; only FORWARD, SUBSCRIBER_PRIORITY and GROUP_ORDER are written as one byte (read on 2026-10-10; no bytes were captured).
