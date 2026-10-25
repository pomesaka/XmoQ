# Objects whose Track Alias is not known yet

A subscriber can read the first Objects of a track before the SUBSCRIBE_OK that gives their Track Alias: the two come on different streams. A relay that held a SUBSCRIBE for a publisher (RENDEZVOUS_TIMEOUT) answers it and passes on that publisher's first Object at once. draft-ietf-moq-transport-18 §11.4.2 lets the receiver choose: "If an endpoint receives a subgroup with an unknown Track Alias, it MAY abandon the stream, or choose to buffer it for a brief period to handle reordering with the control message that establishes the Track Alias."

Both implementations of this repository hold such Objects **while a SUBSCRIBE of theirs waits for its answer**, up to **1 MiB for the session** (provisional), and give them to the subscription once the alias is known, in the order they came. `@xmoq/moqt` does it in `packages/moqt/src/core/incoming-subgroups.ts` (`arrivalOf`, `settleOf`, `heldBytesOf`), and the Rust implementation in `crates/moqt/src/early.rs`, which the terminal viewer, the reader for agents, the gatherer and the transcriber use. Why that choice and that limit is written where the rule is. These rows pin the rule; both read them in a test.

## Format

Tab-separated, no header, one line feed after every row. Column 1 is the name of the row, unique in its file.

`arrival.tsv`: what becomes of an Object.

| Column | Content |
|---|---|
| 2 | What its Track Alias is to the session: `established`, `draining` (PUBLISH_DONE came, streams are awaited), `retired` (the subscription is over), `unknown` |
| 3 | Whether a SUBSCRIBE of the session waits for its answer: `yes`, `no` |
| 4 | The bytes held for the session before it |
| 5 | The bytes it is counted at (`held-bytes.tsv`) |
| 6 | `deliver`, `hold`, `stop-cancelled` (its stream is stopped with `CANCELLED`, 0x1), `stop-internal-error` (with `INTERNAL_ERROR`, 0x0: the limit is this side's) |

`settle.tsv`: what becomes of what is held for an alias after the session took a SUBSCRIBE_OK, or a SUBSCRIBE ended.

| Column | Content |
|---|---|
| 2 | What the alias is to the session now, as above |
| 3 | Whether a SUBSCRIBE still waits for its answer |
| 4 | `release` (to the subscription), `keep`, `stop-cancelled` |

`held-bytes.tsv`: what a held Object is counted at. 64 bytes, its payload, and for each Object Property 16 bytes and the bytes of its value (none for a varint).

| Column | Content |
|---|---|
| 2 | The length of its payload |
| 3 | Its properties, comma-separated: `v` for one with a varint value, `bN` for one whose value is N bytes; `-` for none |
| 4 | The bytes |

## What is not in the tables

- A stream reset while it holds Objects lets go of them; one that ended with its FIN keeps them for its alias (`reset` of `incoming-subgroups.ts`; a test of `crates/moqt/src/session.rs`).
- Held Objects stay on a stream once one of its Objects is held, so that its Objects stay in their order.
- Where a stream is stopped for being over the limit, what it held is let go of and no longer counted.
