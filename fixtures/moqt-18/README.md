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

For RENDEZVOUS_TIMEOUT the draft does not state the value encoding (§10.2.6). XmoQ encodes it as a varint. The `moqtail` crate 0.15.0 does the same: `serialize_message_parameters` in `libs/moqtail-rs/src/model/parameter/message_parameter.rs` of `moqtail/moqtail` at tag `relay@0.15.0` writes its value with `put_vi`, the varint writer; only FORWARD, SUBSCRIBER_PRIORITY and GROUP_ORDER are written as one byte (read on 2026-10-10; no bytes were captured).
