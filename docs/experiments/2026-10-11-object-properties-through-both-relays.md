# Object Properties through both relays

- Date: 2026-10-11
- Commit SHA: the commit that adds `--object-properties` to `interop/run.ts` together with this record. The runs were made on its working copy, on `main` at `a2a3884a`
- Roadmap item: groundwork for Phase 1 (media as LOC). It closes no item

## Objective

Both own implementations read and write Object Properties, and no interop run had carried one. Send an Object with properties through the moqtail relay and through the own relay, and find out two things: that the reading and writing holds with real peers, and **whether the moqtail relay forwards a property it does not know unchanged**, as draft 18 §2.5 asks. If it did not, LOC Objects could not go through that relay.

What the texts say, read before the run:

| Requirement | Section |
|---|---|
| "If a Relay does not support a Property, it MUST NOT be modified, MUST be forwarded, and MUST be cached with the Track or Object" | draft 18 §2.5 |
| Object Properties are a length in bytes, then Key-Value-Pairs | §11.2.1.2 |
| A Key-Value-Pair has its Type as a delta from the previous one; "Length: Only present when Type is odd"; the value is "A single varint encoded value when Type is even, otherwise a sequence of Length bytes" | §1.4.3 |
| Only an Object with status Normal has properties | §11.2.1.2 |
| The PROPERTIES bit is in the type of the Subgroup Header, for the whole stream | §11.4.2 |
| Video Config is property `0x0D`, bytes; Timestamp is `0x10`, an integer | draft-ietf-moq-loc-04 §2.3 |

Expected, decided before the run: the subscriber gets both properties with the types, values and order that were sent, and the payload, through each relay.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4 (23E214), arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relays | own relay at `a2a3884a`; moqtail relay 0.15.0 (`dd8623d`, tag `relay@0.15.0`) |
| Clients | own-browser (`@xmoq/moqt`) as publisher and as subscriber. **No other client took part**: own-rust has no flag for properties and prints none, and the moqtail peers send an empty set |
| The Object | Object 0 of Group 0, alone on its stream, 54 bytes of payload. Property `0x0D` with 18 bytes, property `0x10` with the integer 1760140800123456, which takes 8 bytes. The numbers are LOC-04's for Video Config and Timestamp; the values are made up, and nothing reads them as LOC |

## Commands

```sh
cd interop
bun run.ts --publisher own-browser --subscriber own-browser --relay own --object-properties --await-publish-done
bun run.ts --publisher own-browser --subscriber own-browser --relay moqtail --object-properties --await-publish-done
bun run.ts --publisher own-browser --subscriber own-browser --relay moqtail --object-properties --verbose   # the bytes
mise run check
mise run interop
```

## Observed results

**Both relays forwarded the two properties unchanged.**

| Run | Result |
|---|---|
| Through the own relay | passed at the first run: payload and properties as sent |
| Through the moqtail relay | passed at the first run: the same |
| `mise run check` | passed; 394 TypeScript tests |
| `mise run interop` | 58 of 58 pairings (two more than before, this run through each relay) |

The bytes of the Object, the same on the publisher's stream and on the subscriber's, through both relays (the payload left out):

```
00                      Object ID Delta 0
1d                      Properties Length 29
0d 12 01 42 ... 00 00   Delta Type 13, Length 18, 18 bytes
03 fe 06 40 d6 b7 25 62 40   Delta Type 3 (0x10), the integer in 8 bytes
36 ...                  Payload Length 54
```

**The Subgroup Header.** The publisher sent type `0x5D` (PROPERTIES, explicit Subgroup ID, END_OF_GROUP, FIRST_OBJECT) with Track Alias 1; both relays sent the subscriber `0x5D` with its own Track Alias 0. The bit was passed on as it came.

**Neither relay passes the bytes through.** The own relay decodes each Object and encodes it again (its core holds the properties as a list). The moqtail relay does the same: read at `dd8623d`, `apps/relay/src/server/subscription.rs` turns what it received into an object (`object.try_into_subgroup()`) and writes `sub_object.serialize(previous_object_id, has_properties)`, and `libs/moqtail-rs/src/model/common/pair.rs` writes each property's type as a delta (`serialize_delta`). So the run through the own relay is also the first time the Rust encoding of properties met a real peer, and the run through the moqtail relay has moqtail's parser and encoder agree with the own TypeScript ones.

**A fixture was taken from it**: `fixtures/moqt-18/subgroup-stream-object-properties-from-own-ts.hex`, the stream the moqtail relay wrote to the subscriber. Both test suites decode it and encode it back. It is the first fixture with a property of an odd type and with an integer of 8 bytes in a property. What it is and is not is in the [fixtures' README](../../fixtures/moqt-18/README.md): moqtail's encoding of a value an own client chose, not a property a moqtail client made up.

**The checks fail when they should.** Temporary changes, each taken back, the diff of `crates/` and of the fixture empty afterwards:

| Change | Result |
|---|---|
| The own relay clears the properties of every Object | the run fails: "arrived without Object Properties: they were dropped on the way", with what was sent and what came |
| The own relay keeps the first property only | fails: "arrived with other Object Properties than were sent" |
| The own relay reverses the list | **passes**: the Rust encoder writes properties in ascending order of type whatever the order of the list, as the delta encoding needs. A relay built on it cannot reorder them on the wire |
| One byte of the fixture changed | the decoding and the encoding test fail, in both suites |

### The own Rust client (added later on 2026-10-11, on `main` at `07322d4c`)

The own Rust client got a flag for the properties of the Objects it sends (`--object-property <type>=<value>`, repeated) and prints the ones it receives in its `OBJECT` line (`properties=13:0142...,16:1760140800123456`, or `properties=none`; `payload_hex` stays the last field, which the drivers read to the end of the line). The pairings are now each own client to each own client, four through each relay.

| Run | Result |
|---|---|
| own-browser to own-rust, own-rust to own-browser, own-rust to own-rust; own relay and moqtail relay | all six passed at the first run, with the expectation of the run above unchanged |
| Temporary change to the own relay, taken back: the properties cleared, or the first one kept | the three pairings with the Rust client fail, with the messages above, at the Rust subscriber and at the page |

So the Rust client's writing and reading of properties, which the first run could not show, holds against the own TypeScript client and through moqtail's parser and encoder.

## Limitations and blockers

- The Rust client took no part in the first run; the section above has it.
- **No property came from a moqtail encoder as its origin.** The moqtail peers here send none with content. The moqtail npm library has types for LOC properties (with older numbers); a publisher built on it was not written, because the relay's own parsing and serializing already puts moqtail's encoder into the path.
- One Object a stream, and the subscriber was there before it was sent. **A subscriber that joins while a stream with properties is under way was not run.** The own relay builds that subscriber's header from the one it received and changes two fields of it, neither the PROPERTIES bit (`crates/relay-core/src/lib.rs`, `late_header`); the core's test of that header (`tests.rs`, the subscriber that joins inside Group 3) uses a stream without the bit, so no test would notice the bit being dropped there.
- Properties on an Object that is not Normal, a property the draft defines, more than two properties, and the cache (§2.5's third MUST; neither relay was asked for a FETCH) are outside this run.
- Whether the three ways a relay could fail are told apart was tried for two of them (dropped, changed). The third, an Object that does not come, has a message with the page's state (session open or closed, Objects read) and was not provoked.

## Open questions raised

- Should the core's test of a late subscriber's header use a stream with the PROPERTIES bit?
