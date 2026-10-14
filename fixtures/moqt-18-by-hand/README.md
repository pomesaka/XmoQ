# Bytes for draft 18 written by hand

**Nobody's implementation wrote these bytes.** They were written by hand from the text of `draft-ietf-moq-transport-18`, for parts of a fetch stream (§11.4.4) of which no capture exists, and the person who read the draft is the one who wrote down what they decode to. They are no check against another implementation. What they check is that the Rust and the TypeScript implementation read the same bytes as the same values, and write these bytes from those values.

Bytes another implementation put on the wire are in [`fixtures/moqt-18/`](../moqt-18/README.md). The two are kept apart because a test of each implementation holds that every file there is a capture.

Variable-length integers are those of §1.4.1, not of QUIC: `80 8c` is 0x8C and `81 0c` is 0x10C.

| File | Group Order | Decodes to | Built from |
|---|---|---|---|
| `fetch-stream-descending` | Descending | FETCH_HEADER, Request ID 4. Objects {9, 0}, {7, 1}, {6, 5}, each in Subgroup 0 with Publisher Priority 7 and one byte of payload (`aa`, `bb`, `cc`) | §11.4.4.1: "If the Group Order is Descending, the Group ID is the prior Object's Group ID minus the (Group ID Delta + 1)", and "When the Group ID Delta field is present, the Object ID is the value of Object ID Delta" |
| `fetch-stream-end-of-range` | Ascending | FETCH_HEADER, Request ID 0. Object {0, 0}; End of Non-Existent Range at {0, 5}; Object {0, 6}; End of Unknown Range at {2, 0}; Object {2, 3}. Subgroup 0, priority 7, payloads `aa`, `bb`, `cc` | §11.4.4.2: the two fields of an indicator are a Location, and "Prior Group ID and prior Object ID: The values from the End of Range indicator" |
| `fetch-stream-indicator-first` | Ascending | FETCH_HEADER, Request ID 0. End of Unknown Range at {3, 1}; Object {3, 2}, Subgroup 0, priority 7, no payload | §11.4.4.2, for an indicator that no Object came before |
| `fetch-stream-indicator-first-group-delta` | Ascending | FETCH_HEADER, Request ID 0. End of Unknown Range at {3, 1}; Object {9, 0}, Subgroup 0, priority 7, no payload | The same, with a Group ID Delta that counts from the indicator |
| `fetch-stream-subgroup-id-modes` | Ascending | FETCH_HEADER, Request ID 0. Object {1, 0} without a Subgroup ID (the Datagram bit), priority 7; Object {2, 0} in Subgroup 4, priority 9; Object {2, 1} in Subgroup 5, priority 9. No payloads | §11.4.4.1, Table 8 and the bit 0x40 |

Control messages, one a file:

| File | Decodes to | Built from |
|---|---|---|
| `request-update-request-id-3` | REQUEST_UPDATE, Request ID 3, no parameters | §10.9, Figure 12: Type `0x2`, a Length of 16 bits, Request ID, Number of Parameters. No capture of a REQUEST_UPDATE exists: `moqtail`'s clients send none |
| `request-update-request-id-5` | The same with Request ID 5 | The same |
| `request-update-new-group-request` | REQUEST_UPDATE, Request ID 3, NEW_GROUP_REQUEST 7 | §10.2.13: "The NEW_GROUP_REQUEST parameter (Parameter Type 0x32) is a varint"; §10.2, Figure 4: the first parameter's Type Delta is its Type. `02 00 04 03 01 32 07`: a Length of 4, Request ID 3, one parameter, Type 0x32, value 7. `moqtail`'s clients send no NEW_GROUP_REQUEST |

`02 00 02 03 00`: the type, a Length of 2 for the two bytes that follow, Request ID 3, no parameters. [`fixtures/peer-subscribe/`](../peer-subscribe/README.md) has what the two are sent for.

The Group Order is that of the FETCH the stream answers. The bytes do not carry it, and the same bytes decode to other Group IDs under the other order.

## Byte by byte

`fetch-stream-descending`: `05 04` is the header. `1c 09 00 07 01 aa`: Serialization Flags 0x1C (Group ID, Object ID and priority present; Subgroup ID mode 0, which is Subgroup 0), Group 9, Object 0, priority 7, one byte of payload. `0c 01 01 01 bb`: flags 0x0C, Group ID Delta 1, so Group 9 - (1 + 1) = 7; Object ID 1; the priority of the prior Object. `0c 00 05 01 cc`: Group ID Delta 0, so Group 6; Object ID 5.

`fetch-stream-end-of-range`: `05 00`, then `1c 00 00 07 01 aa`, Object {0, 0}. `80 8c 00 05`: End of Non-Existent Range at {0, 5}. `00 01 bb`: flags 0x00, so the prior Group, and the prior Object ID plus one; the prior ones are the indicator's, so {0, 6}. `81 0c 02 00`: End of Unknown Range at {2, 0}. `04 03 01 cc`: flags 0x04, Object ID Delta 3 added to the indicator's Object ID in its Group: {2, 3}. The Subgroup and the priority of the last two Objects are those of the first.

`fetch-stream-indicator-first`: `05 00`, `81 0c 03 01`, then `10 07 00`: flags 0x10, only the priority present. Group 3 and Object 1 + 1 from the indicator.

`fetch-stream-indicator-first-group-delta`: `05 00`, `81 0c 03 01`, then `1c 05 00 07 00`: Group ID Delta 5 from the indicator's Group 3 is Group 3 + 5 + 1 = 9; Object ID 0.

`fetch-stream-subgroup-id-modes`: `05 00`, then `5c 01 00 07 00`: flags 0x5C, which has the Datagram bit 0x40, so no Subgroup ID; Group 1, Object 0, priority 7. `1f 00 04 00 09 00`: flags 0x1F, Group ID Delta 0 (Group 2), Subgroup ID 4 present, Object 0, priority 9. `02 00`: Subgroup ID mode 2, the prior Subgroup ID plus one, 5; Object 1.

## Where the reading is not settled

**The two `indicator-first` files depend on a place where the draft says two things.** §11.4.4.1 has "The first Object MUST include a Group ID Delta and Object ID Delta, and these values are the absolute Group ID and Object ID". By that, the Object of `fetch-stream-indicator-first` is a violation, and that of `fetch-stream-indicator-first-group-delta` is in Group 5. The files follow §11.4.4.2. The decision and its basis are in the [implementation notes](../../docs/moqt-implementation-notes.md) (row on the first Object of a fetch stream when an indicator came before it), marked provisional; `moqtail` 0.15.0 reads it the other way.

Reading the two fields of an indicator as a Location, and not as deltas, is provisional as well (row on the two fields of an End of Range indicator). All files but `fetch-stream-descending` and `fetch-stream-subgroup-id-modes` depend on it.

Input that has to be refused is not here. Each implementation has those bytes in its own tests.
