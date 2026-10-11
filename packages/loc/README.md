# @xmoq/loc

The Low Overhead Media Container, written from the text of `draft-ietf-moq-loc-04`. Section numbers below are of that draft unless they say "transport", which is `draft-ietf-moq-transport-18`.

Status: partly implemented. What is here was run against bytes of `moqlivemock`'s publisher and, for the video payload, against the H.264 decoder of Chromium 156. Nothing encodes or plays media yet.

## Scope

Implemented:

- The time of a frame as Object Properties: `locProperties` makes the Timestamp (and the Timescale, where there is one) of a frame, `readLocTiming` reads them out of the properties of an Object, `microsecondsOf` gives the time in the unit of WebCodecs.
- `toAnnexB`: a video payload that has a length before every NAL unit becomes one with start codes, which is the form a decoder of WebCodecs takes without a description.

Outside this package: how frames become Objects and Groups, which MSF-01 says for a LOC track (§4.1, one sample an Object and a group of pictures a Group; §6, an Object a stream). That is for what sends and receives media.

Not implemented: Video Config, Video Frame Marking, Audio Config and Audio Level (not read, not written), private properties in the payload (§2.2), encryption (§3), a start code of 3 bytes, the other way of `toAnnexB`.

## Layer rules

Everything is bytes and numbers: no I/O, and **no other package of this repository**. `@xmoq/loc` does not import `@xmoq/moqt`: an Object Property is taken and given as a type and a value, the shape of a Key-Value-Pair there. `tsconfig.core.json` type-checks `src/core` without the DOM library, `noRestrictedImports` in the root `biome.jsonc` keeps the imports out, and `scripts/ci/check-layers.sh` the dependencies: `@xmoq/moqt` and `@xmoq/loc` are the lowest layer and know nothing of each other.

## How the draft is read

| Point | What is done | From |
|---|---|---|
| Which revision of LOC, on which transport | The numbers of LOC-04 on transport draft 18. **Provisional** ([open question](../../docs/open-questions.md)): MSF-01 cites LOC-02, and LOC-04 cites transport draft 19. The one revision that cites draft 18 is LOC-03, which has two numbers for the Timestamp, `0x0A` in §2.3.1.1 and `0x06` in the table of its IANA section | The table below: no number of LOC-04 is one transport draft 18 defines. `moqlivemock`'s publisher sends `0x10` for the Timestamp on draft 18 (`fixtures/loc-04/`) |
| Which properties an Object has | The Timestamp, and the Timescale where the time is not the wall clock. No property is required | §2.2: "The LOC Public and Private Properties carry optional metadata related to the Payload". None of the six definitions has a MUST |
| The Timestamp without a Timescale | Microseconds since the Unix epoch | §2.3.1.1: "If no timescale property is present, the timestamp is interpreted as wall-clock time in microseconds since the Unix epoch." |
| The order of the properties that are written | Timescale (`0x08`), then Timestamp (`0x10`) | MoQ Transport writes the types of a list of Key-Value-Pairs as differences, in ascending order (transport §1.4.3) |
| A Timestamp or a Timescale twice, a Timescale of 0, a Timescale without a Timestamp | `invalid` | **The draft says nothing of a receiver.** A second value of one property has no meaning the draft gives; a Timescale of 0 has no unit; a Timescale alone gives a unit to nothing |
| An Object without a Timestamp | Read as that: no time | §2.2, "optional" |
| A Timescale that is a property of the track | **Not read.** `readLocTiming` is given the properties of an Object, and a Timescale alone is `invalid` there. A caller that has the track's Timescale puts it into the timing it got | §6.1 has TIMESCALE with the scope "Track, Object". §2.3.1.1, "If no timescale property is present", does not say whether that is of the Object or of the track as well. **Without the track's value a Timestamp reads as wall-clock time**: 3000 of a track with 90000 a second is 3000 microseconds, where it is 33333 |
| A value that MoQ Transport cannot carry (below 0, 2^64 and above) | `locProperties` throws a `RangeError`. `readLocTiming` hands out what it was handed | The integers are the decoder's of the transport; they are not checked a second time |
| The other properties of LOC, and properties LOC does not define | Passed over | Not read here. A relay passes a property it does not know unchanged (transport §2.5) |
| A video payload: lengths or start codes | The four bytes where a NAL unit begins are a number: 1 is a start code, and from there to the end the payload is left as it is; any other number is the length of the NAL unit, and becomes a start code. The result is a new array; the payload that was given is not written to | §2.1.3: "A 4-byte length prefix can be sent before each NAL Unit ... A length value of 1 SHOULD be interpreted as a start code rather than a length." §2.1.4: "A 4-byte start code can be sent before each NAL Unit ... The start code value is 1 in network byte order". **That the rest is left alone after a start code is read into the draft, which says nothing of a payload with both** |
| A length of 1 with lengths after it | Taken for a start code, as the SHOULD has it: the lengths after it stay lengths, and the result is `ok` | §2.1.3, the same sentence. Whether H.264 has a NAL unit of one byte was not read from its specification, so this is not made `invalid` |
| A length of 0, a length beyond the payload, fewer than 4 bytes where a length is to begin | `invalid`. A start code with nothing after it is `ok`: what follows a start code is not looked into, so the two are not treated alike | **The draft says nothing of a receiver.** A length that does not fit is no length; a NAL unit of no bytes is taken for a fault |
| The bytes of a NAL unit | Copied as they are; whether something in them looks like a start code is not looked at | The draft leaves the content of a NAL unit to the codec. What keeps a start code from occurring inside one is in the codec's specification, which was not read for this |
| A start code of 3 bytes | Not known: its first four bytes read as a length | §2.1.4: "A 3-byte start code, which is uncommon, MAY be used if the track never uses length prefixes or any Video Config". Nothing in a payload says which the track uses, so a caller that knows its track has start codes does not call `toAnnexB` |
| A length prefix of fewer than 4 bytes | Not read | §2.1.3: it "MAY be specified in the Video Config", which is not read |

### The numbers of the properties

| LOC-04 (§6.1) | | Transport draft 18 defines (§15.8) | |
|---|---|---|---|
| `0x08` | TIMESCALE | `0x02` | OBJECT_DELIVERY_TIMEOUT |
| `0x09` | VIDEO_FRAME_MARKING | `0x04` | MAX_CACHE_DURATION |
| `0x0C` | AUDIO_LEVEL | `0x06` | SUBGROUP_DELIVERY_TIMEOUT |
| `0x0D` | VIDEO_CONFIG | `0x0B` | IMMUTABLE_PROPERTIES |
| `0x0F` | AUDIO_CONFIG | `0x0E` | DEFAULT_PUBLISHER_PRIORITY |
| `0x10` | TIMESTAMP | `0x22`, `0x30`, `0x3C`, `0x3E` | DEFAULT_PUBLISHER_GROUP_ORDER, DYNAMIC_GROUPS, PRIOR_GROUP_ID_GAP, PRIOR_OBJECT_ID_GAP |

- No number is in both columns, and all of LOC-04's are below `0x4000`, where the Mandatory Track Properties of transport §2.5.1 begin.
- **The same section of transport draft 18 has a second table, of registrations it expects from LOC, with other numbers**: TIMESTAMP `0x06`, TIMESCALE `0x08`, VIDEO_FRAME_MARKING `0x0A`, AUDIO_LEVEL `0x0C`, VIDEO_CONFIG `0x0D`. Its `0x06` is the SUBGROUP_DELIVERY_TIMEOUT of the first table. **No revision of LOC, from -00 to -04, has that set of numbers**: `0x0A` is the Video Frame Marking in none (4 up to -02, `0x09` from -03), and the Timestamp is 2 up to -01, `0x06` in -02, `0x0A` and `0x06` in -03, `0x10` in -04. They are not used here.

## Known gaps

- **The Timescale of a track is not read** (the table above). What receives media needs a way to hand the track's value in.
- **A payload with start codes of 3 bytes can come out `ok` and broken.** Its first four bytes are read as a length; where that happens to be the number of bytes that follow, the fourth byte, the first of the NAL unit, is overwritten: `00 00 01 09` and 265 bytes becomes `00 00 00 01` and those 265 bytes. Every other such payload is `invalid`. A track with start codes of 3 bytes is not given to `toAnnexB`.
- **No Timescale that another implementation sent was read**: `0x08` is written here and read here. The Timestamp is in the bytes of `moqlivemock`.
- **Nothing checks that `LocProperty` is the shape of `KeyValuePair` of `@xmoq/moqt`**, and no test hands the properties of the captured stream to `readLocTiming`: neither package can import the other. It is for the package that uses both.
- **A video payload is not checked to be video.** `toAnnexB` knows lengths and start codes, and nothing of what a NAL unit is: it cannot tell a key frame from another, nor say that parameter sets are missing.
- The time of a frame is carried and not compared: nothing here says that timestamps grow.
- `microsecondsOf` rounds down, so two frames closer than a microsecond in a fine timescale get the same time.

## Checked against

`fixtures/loc-04/` has the start of a video stream of `moqlivemock`'s publisher: its two payloads are read by `toAnnexB`, and the stream as a whole by the decoder of `@xmoq/moqt` (a test there), not by Rust yet. After `toAnnexB`, the decoder of Chromium 156 took the key frame and the frame after it, and refused them as they came ([record](../../docs/experiments/2026-10-11-loc-video-payloads-in-chromium.md)).
