# When a stream of the peer has said what it is

Both implementations stop a stream the peer opens when it has not said what it is in time (10 seconds, `first_bytes_timeout` and `firstBytesTimeoutMs`), and leave one that has. The draft has no such time, so where "has said what it is" lies is a contract of XmoQ's own, and these rows pin it: the bytes a stream has sent when its time runs out, and whether it is stopped.

## Format

`said.tsv`, one row per stream, four columns separated by tabs, no header. Every row ends with one line feed, the last one too; there are no empty rows. Hexadecimal digits are lower case.

| Column | Content |
|---|---|
| 1 | Name of the row; unique together with column 2 |
| 2 | `uni` or `bidi`: a unidirectional stream of the peer, or a bidirectional one |
| 3 | Every byte the stream has sent when its time runs out, in hexadecimal digits. Empty for a stream that sent nothing |
| 4 | `said`: the stream is left alone. `not-yet`: it is stopped, and the session stays open |

What the rows hold a reader to:

| Stream | Has said what it is with |
|---|---|
| Control stream, padding stream | its Stream Type |
| Subgroup stream | its whole Subgroup Header |
| Fetch stream | its whole FETCH_HEADER |
| Request | its first message, whole |

A stream that is stopped gets STOP_SENDING `INTERNAL_ERROR` (`0x0`) where it is unidirectional. A request is given up with `CANCELLED` (`0x1`): STOP_SENDING for the peer's direction, and a reset of the other one where nothing was written on it. The codes are not in the table.

## What a test has to set up

- The receiver is the client of the session, so the request of the `bidi` rows has Request ID 1, one of the server's.
- The `fetch-header` row names Request ID 0. A reader that asks its session what a fetch stream belongs to needs a FETCH with that ID open; with none, the stream closes the session, which is another matter than this table.
- The subgroup rows are for Track Alias 0, which need not be known: a stream for an alias no subscription has is stopped for that reason after its header, and has said what it is all the same.

## Source

**The second and third column were written for this table, and the fourth is a decision, not an observation.** No other implementation was asked: the time is XmoQ's own. The decision is the row "A stream of the peer that does not say what it is" in [the implementation notes](../../docs/moqt-implementation-notes.md), and it follows where the Rust reader cleared its deadline when the table was written (`crates/moqt-io/src/streams.rs`).

Where the bytes come from:

- `af 00` is the start of a SETUP, `f0 13 2b 3e 28` the padding type 0x132B3E28 (§11.5.1), `05` the type of a FETCH_HEADER and `80` the first byte of a type that takes two (§1.4.1).
- `5d 00 00 00 00` is a Subgroup Header of type 0x5D: Track Alias 0, Group 0, Subgroup 0, Publisher Priority 0. `00 05 61` after it is the start of an Object, written for this table and found in no capture: Object ID Delta 0, then, since 0x5D has the PROPERTIES bit (§11.4.2), an Object Properties Length of 5, of which one byte has come.
- The request is the SUBSCRIBE of [`fixtures/moqt-18/subscribe-moqtail-rs.hex`](../moqt-18/README.md) with its Request ID changed from 0 to 1, cut at six lengths.

`said.tsv` is written by `generate.ts` next to it, from that capture and the bytes above, not by hand: `bun fixtures/first-bytes/generate.ts`.
