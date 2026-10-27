# Objects whose Track Alias is not known yet

A subscriber can read the first Objects of a track before the SUBSCRIBE_OK that gives their Track Alias: the two come on different streams. A relay that held a SUBSCRIBE for a publisher (RENDEZVOUS_TIMEOUT) answers it and passes on that publisher's first Object at once. draft-ietf-moq-transport-18 §11.4.2 lets the receiver choose: "If an endpoint receives a subgroup with an unknown Track Alias, it MAY abandon the stream, or choose to buffer it for a brief period to handle reordering with the control message that establishes the Track Alias."

Both implementations of this repository hold such Objects **while a SUBSCRIBE of theirs waits for its answer**, up to a limit for the session, and give them to the subscription once the alias is known, in the order they came: the tables below, and `early-objects-bytes` of [`fixtures/limits/`](../limits/README.md). These rows pin the rule; both read them in a test.

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

`held-bytes.tsv`: what a held Object is counted at. `early-object-overhead-bytes`, its payload, and for each Object Property `early-property-overhead-bytes` and the bytes of its value (none for a varint), the two of [`fixtures/limits/`](../limits/README.md).

| Column | Content |
|---|---|
| 2 | The length of its payload |
| 3 | Its properties, comma-separated: `v` for one with a varint value, `bN` for one whose value is N bytes; `-` for none |
| 4 | The bytes |

`reused-alias.tsv`: the rule of `settle.tsv` in a session, for a Track Alias the publisher gives to another track once a subscription of it is over (§11.1 forbids only two tracks "simultaneously"). Two SUBSCRIBEs wait; an Object of alias 7 is held; the first SUBSCRIBE_OK names a Mandatory Track Property, so its subscription ends at once and its alias is retired; the second SUBSCRIBE_OK gives alias 7.

| Column | Content |
|---|---|
| 2 | The stream of the held Object before the first SUBSCRIBE_OK: `open`, or `fin` (it ended with its FIN) |
| 3 | The Track Alias of the first SUBSCRIBE_OK, the one that is retired |
| 4 | What the second subscription gets of the held Object: `given`, `none` |

## What is not in the tables

- A stream reset while it holds Objects lets go of them; one that ended with its FIN keeps them for its alias, by the rule of `settle.tsv` as a stream still open (`reused-alias.tsv` has such rows).
- Held Objects stay on a stream once one of its Objects is held, so that its Objects stay in their order.

`aliases.tsv`: steps in a client's session, and then whom a Track Alias names. The Rust and the TypeScript session read it.

| Column | Content |
|---|---|
| 2 | The steps, in order, separated by spaces: `subscribe:<t>`, a SUBSCRIBE of this end for the track `t` of the namespace (`xmoq`); `ok:<t>:<alias>`, its SUBSCRIBE_OK with that Track Alias; `ok-mandatory:<t>:<alias>`, the same with the Mandatory Track Property 0x4000 (§2.5.1); `done:<t>:<count>`, PUBLISH_DONE on its stream with that Stream Count; `end:<alias>`, a subgroup stream under that alias that ends after its header; `publish:<alias>`, a PUBLISH of this end with that alias; `peer-subscribe:<t>`, a SUBSCRIBE of the peer, a server, that this end accepts |
| 3 | `object:<t>`: an Object under alias 9 goes to the subscription of track `t`. `stopped`: its stream is stopped. `closed:<code>`: a step closed the session with that Session Termination Error Code (§3.5). `given:<aliases>`: the aliases this end gave the SUBSCRIBEs of the peer it accepted, in order |
