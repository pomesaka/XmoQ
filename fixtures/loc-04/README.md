# A LOC video stream of another implementation

The start of a video stream of `mlmpub`, the publisher of [`Eyevinn/moqlivemock`](https://github.com/Eyevinn/moqlivemock) v0.16.1 (commit `91d903e9933a452f8baaa860f9fcd20cf195e077`), as an own client received it: the track `video_400kbps_avc` of the namespace `mlm/msf/clear`, H.264 (`avc3.4D401F` in its catalog), 1280x720, 25 frames a second. `@xmoq/loc` reads the payloads. The stream they came on is bytes of MoQ Transport and lives with the other streams, in [`fixtures/moqt-18/`](../moqt-18/README.md), where the decoders of `@xmoq/moqt` and of `crates/moqt` read it; both take its two payloads from here. No Rust code reads LOC yet; one that does reads these files.

Taken on 2026-10-11, with `mlmpub` as `scripts/ci/interop.sh` builds it (unchanged) and the own Rust client of `main` at `36e51810`:

```sh
(cd <source of v0.16.1>/cmd/mlmpub && interop/peers/moqlivemock/.tools/bin/mlmpub -addr 127.0.0.1:45380 -sideport 46380)
xmoq-moqt-client --mode subscribe --objects 27 --url https://127.0.0.1:45380/moq \
    --cert-sha256 "$(curl -s http://127.0.0.1:46380/fingerprint)" \
    --namespace mlm/msf/clear --track video_400kbps_avc --trace
```

The client printed the bytes of the stream as it read them (`TRACE recv uni#1 ...`) and every Object it decoded (`OBJECT group=... payload_hex=...`). The files are cut from the first, and compared with the second.

| File | Bytes | What it is |
|---|---|---|
| `../moqt-18/subgroup-stream-two-objects-from-moqlivemock.hex` | 22,500 | The first bytes of the first subgroup stream the client got, up to the end of its second Object. The stream went on: `mlmpub` sends a Group on one stream, 25 Objects. Both implementations decode it to the values below and encode those to the same bytes (`packages/moqt/test/core/moqtail-fixtures.test.ts`, `crates/moqt/src/data.rs`) |
| `video-key-length-prefixed-from-moqlivemock.hex` | 21,533 | The payload of the first Object, a key frame |
| `video-delta-length-prefixed-from-moqlivemock.hex` | 931 | The payload of the second Object |

## Decoded

The stream: `15 00 f0 6a cb 07 1f 00 80` is the SUBGROUP_HEADER, type `0x15` (Object Properties present, an explicit Subgroup ID, a Publisher Priority; neither the end of the Group nor the first Object is marked), Track Alias 0, Group 1791690527, Subgroup 0, Publisher Priority 128. The Group ID is the wall-clock time in seconds.

| Object | Object Properties | Payload |
|---|---|---|
| 0 | 9 bytes, `10 fe 06 5d 88 75 68 c5 c0`: type `0x10`, the Timestamp of LOC-04, with the integer 1791690527000000, the wall-clock time in microseconds | 21,533 bytes, the key frame file |
| 1 | 9 bytes, `10 fe 06 5d 88 75 69 62 00`: the same type, 1791690527040000, 40 ms later | 931 bytes, the other file |

The 27 Objects that were read had no other property. The first Object of the next Group had the Timestamp 1791690528000000.

**The payloads have a length of 4 bytes before every NAL unit, and no start code** (the catalog says `avc3`, and no track has initialization data):

| Payload | NAL units, as length and first byte |
|---|---|
| The key frame | 25 bytes, `67` (a sequence parameter set); 4 bytes, `68` (a picture parameter set); 21,492 bytes, `65` (a slice of an IDR picture) |
| The other frame | 927 bytes, `41` (a slice of another picture) |

What the first byte of a NAL unit means is H.264's and was not read from its specification for this: the three names are what the decoder of Chromium needs in a key frame, which took these two frames once every length was a start code ([record](../../docs/experiments/2026-10-11-loc-video-payloads-in-chromium.md)).
