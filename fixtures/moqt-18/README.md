# MOQT draft-18 wire fixtures

Bytes that moqtail put on the wire, and one stream of `moqlivemock`'s publisher, used as fixed expectations by XmoQ's own implementations. A codec that only decodes what it encoded itself cannot notice that both directions misread the draft in the same way; these files come from another implementation.

## Format

One file per control message or per stream. The content is hexadecimal digits; spaces and newlines are ignored. A control message file holds the whole message, including Message Type and Message Length. A subgroup stream file holds the stream from its first byte: the Subgroup Header, then the Objects.

The tests of both implementations read every file, decode it into the value they hold as their expectation, and encode that value back into the same bytes.

## Source

All bytes were captured on 2026-10-10 on macOS 14.4 arm64 over loopback, against the moqtail relay 0.15.0 (`moqtail/0.15.0+dd8623d`, tag `relay@0.15.0`). Namespace `xmoq/phase0`, track `object`.

Three ways of capturing were used.

- **Relay log.** The relay prints a hex dump for each control message, such as `Subscribe (34 bytes): 03 00 1f ...`. These are the bytes on the wire, not a re-encoding: for a received message the relay prints the slice of its receive buffer that the message occupied, and for a sent message it prints the buffer it writes to the stream next. Read in `libs/moqtail-rs/src/transport/control_stream_handler.rs` of `moqtail/moqtail` at tag `relay@0.15.0` (`send` and `next_message`).
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
| `subgroup-stream-two-objects-from-moqlivemock.hex` | `mlmpub`, the publisher of `Eyevinn/moqlivemock` v0.16.1 (`91d903e9`), with no relay between: not moqtail, and not the relay above | client trace | capture O, on 2026-10-11 |
| `fetch-standalone-moqtail-rs.hex` | moqtail's test client 0.15.0 | its log | capture I |
| `fetch-ok-moqtail-relay.hex` | relay, answering that FETCH | log of the test client | capture I |
| `fetch-relative-joining-moqtail-rs.hex` | moqtail's test client 0.15.0, after a SUBSCRIBE | its log | capture J |
| `fetch-ok-joining-moqtail-relay.hex` | relay, answering that FETCH | log of the test client | capture J |
| `fetch-stream-from-moqtail-relay.hex` | relay, answering the FETCH of capture I sent again | capture tool | capture K |
| `fetch-stream-five-groups-from-moqtail-relay.hex` | relay, answering an Absolute Joining FETCH written by hand | capture tool | capture L |
| `fetch-stream-gaps-from-moqtail-relay.hex` | relay, answering a FETCH written by hand for a track with gaps in its Object IDs and Group IDs | capture tool | capture M |
| `fetch-stream-empty-from-moqtail-relay.hex` | relay, answering a FETCH written by hand whose range it takes to hold nothing | capture tool | capture N |
| `track-status-moqtail-relay.hex` | relay, passing on to the publisher of the namespace a TRACK_STATUS for a track it did not have | relay log | capture P, on 2026-10-24 |
| `request-ok-track-status-moqtail-relay.hex` | relay, answering a TRACK_STATUS for a track it had | relay log | capture P, on 2026-10-24 |

What a file holds that the values in the tests do not show:

- `subgroup-stream-joined-mid-group-from-moqtail-rs`: its header type 0x5D has FIRST_OBJECT set although Object ID 4 is not the first Object published in the Subgroup ([record](../../docs/experiments/2026-10-10-own-rust-client-vs-moqtail.md)).
- `subgroup-stream-object-properties-from-own-ts`: the property types 0x0D and 0x10 are none that draft 18 defines; they are the numbers draft-ietf-moq-loc-04 gives Video Config and Timestamp, and the values are made up.
- `fetch-ok-joining-moqtail-relay`: the SUBSCRIBE_OK before it had LARGEST_OBJECT = {6, 1}.
- `fetch-stream-five-groups-from-moqtail-relay`: the SUBSCRIBE_OK of that run had LARGEST_OBJECT = {5, 2} (`04 00 05 00 01 09 05 02`) and the FETCH_OK the End Location {6, 1} (`18 00 04 00 06 01 00`); its last Object, {6, 0}, lies after the Joining Location, where draft 18 ends a Joining FETCH (divergence a of [the record](../../docs/experiments/2026-10-11-moqtail-relay-fetch.md)).
- `fetch-stream-gaps-from-moqtail-relay`: the End Location asked for was {4, 0}, and nothing of Group 4 is there.

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

**The requests of L, M and N were written by hand**, from the two FETCH messages of captures I and J with fields changed, and are no fixtures: an Absolute Joining FETCH with Request ID 2, Joining Request ID 0, Joining Start 2 and SUBSCRIBER_PRIORITY 200; and Standalone FETCHes for {0, 0} to {4, 0} and for {1, 0} to {1, 0}. A test of `crates/moqt` decodes each and compares it with the request that was meant.

**What came back differs from draft 18 in places** ([record](../../docs/experiments/2026-10-11-moqtail-relay-fetch.md); the row on how the `moqtail` relay 0.15.0 answers a FETCH, under Other observations of the [implementation notes](../../docs/moqt-implementation-notes.md#other-observations-about-the-draft)). The files are fixtures for how the bytes are laid out, and not for which Objects a FETCH is to return.

Capture P is of TRACK_STATUS (§10.14), with the relay as for G and the own Rust client (`xmoq-moqt-client --mode track-status`; moqtail's test client sends none): one client publishes the track `object` with PUBLISH, then one sends TRACK_STATUS for `object`, and one for `nobody-published-this`. The relay answered the first itself and passed the second on to the publisher, the session that had announced `xmoq/phase0`, under its own Request ID 1; the publisher refused it with `NOT_SUPPORTED`, which the relay passed back. The two files are the relay's lines `TrackStatus (40 bytes)` sent to the publisher and `RequestOk (7 bytes)` sent to the first client. **The relay answers `LARGEST_OBJECT` {0, 0} also before the publisher has sent an Object**, and the same after it sent Object 0 of Group 0; it puts no Track Properties in the answer, where §9.6 has a relay include "all Properties associated with a Track" in TRACK_STATUS_OK (the publisher's PUBLISH had none here). Read in `apps/relay/src/server/message_handlers/track_status_handler.rs` of `moqtail/moqtail` at tag `relay@0.15.0` (`handle`): a track the relay has is answered from it with `LARGEST_OBJECT` alone; otherwise the first publisher of the namespace is asked, and `DOES_NOT_EXIST` is the answer where there is none.

**Capture O is the one file no code of moqtail wrote.** It is the start of a video stream of `mlmpub`, received by the own Rust client, which printed the bytes as it read them; the commands, the revisions, and the two payloads are in [`fixtures/loc-04/`](../loc-04/README.md). The file is the first 22,500 bytes of the first subgroup stream: its header and two Objects of the 25 the stream went on to carry, so it does not end where the stream did.

**The file of capture H is moqtail's encoding of a value an own client chose.** No moqtail client in this repository sends an Object Property with content, so the two properties came from the own browser client. The relay does not pass the bytes through: it parses each Object into a list of properties and serializes it again with moqtail's encoder, the type of each property as a delta from the one before. Read at `dd8623d` in `apps/relay/src/server/subscription.rs` (`object.try_into_subgroup()`, then `sub_object.serialize(previous_object_id, has_properties)`) and `libs/moqtail-rs/src/model/common/pair.rs` (`serialize_delta`). So the file shows that moqtail's parser took what the own encoder wrote and that moqtail's encoder writes the same bytes for it; it does not show a property moqtail made up itself.

Some values depend on the run and will differ when a capture is repeated: the Track Alias moqtail-ts picks at random (779698128977928661 in `publish-moqtail-ts`), the timestamp and nonce in the two 53-byte payloads, the 8 random payload bytes of capture G, and the Group that a subscriber happens to join in capture G. A repeated capture is a new fixture, not a check of the old one.

The subgroup streams are what the relay wrote, not what the publisher wrote: the relay re-serializes the Subgroup Header with the subscriber's Track Alias. The streams of captures D and E had not been closed when they were captured, so those files end after the first Object. The files of capture G hold what had arrived when the subscriber left; whether a FIN followed was not observed.

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

The value of RENDEZVOUS_TIMEOUT, which the draft does not encode, is in [Encodings the draft does not give](../../docs/moqt-implementation-notes.md#encodings-the-draft-does-not-give).
