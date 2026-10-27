# What becomes of a FETCH the peer sends

A publisher receives a FETCH on a bidirectional stream the peer opened: a relay asks it for a range of a track. The session of each implementation decides, before the application sees the request, whether it is handed on, refused, or a reason to close the session; and once the application has the request, what is sent for each thing that happens to it. Both implementations read the two tables here in their tests, so that the same bytes in the same state get the same answer from both.

An End Location that names the same Location as the Start Location is taken (§10.12.3 asks for "the same or a larger Location"), in the FETCH and in the answer.

## `opened.tsv`

One row a line, five fields separated by tabs:

| Field | Meaning |
|---|---|
| name | What the row is about |
| before | What happened in the session before the row's bytes: as in [`../peer-subscribe/README.md`](../peer-subscribe/README.md), and `subscribe-accepted-forward-0`, as `subscribe-accepted` with the SUBSCRIBE's FORWARD 0 in place of 1 |
| kind | `standalone` or `joining`: what the FETCH is, for the one place the two implementations part (below); `neither` for the row with a Fetch Type the draft does not have |
| sent | The bytes the server sends on a new bidirectional stream, in hex: one FETCH, whole |
| outcome | `handed`: the caller of the session gets the request. `refused 0x11`, `refused 0x32`: the session answers REQUEST_ERROR with that code itself (`INVALID_RANGE`, `INVALID_JOINING_REQUEST_ID`). `closed 0x3`, `closed 0x4`: the session is closed with that Session Termination Error Code (`PROTOCOL_VIOLATION`, `INVALID_REQUEST_ID`) |

**Where the two part, by design**: a `joining` row with the outcome `handed`. The Rust session hands a Joining FETCH that names an open SUBSCRIBE of the peer to its caller. `@xmoq/moqt` refuses it itself with `NOT_SUPPORTED` (`0x3`): a relay turns a viewer's Joining FETCH into a Standalone one before it asks a publisher, so a publisher behind a relay gets none, and the Rust client refuses one in the same way. The test of `@xmoq/moqt` reads `handed` in such a row as `refused 0x3`.

## `updates.tsv`

What happens to one Standalone FETCH of the peer, in order, and what this endpoint sends for each step. The FETCH has Request ID 1, Start Location {3, 4} and End Location {5, 1}. One row a line, three fields separated by tabs:

| Field | Meaning |
|---|---|
| name | What the row is about |
| steps | The steps, separated by spaces |
| sent | For each step, in the same order and separated by ` \| `, what it sends: the messages in order, `-` for nothing, `usage-error` for a mistake of the caller that sends nothing and changes nothing, or `closed 0x3` |

| Step | What happens |
|---|---|
| `update` | The peer sends REQUEST_UPDATE with no parameter, with its next Request ID (3, then 5) |
| `update-priority` | The same with SUBSCRIBER_PRIORITY 200, which the update of a FETCH may carry |
| `update-forward` | The same with FORWARD 1, which it may not (§10.2.12) |
| `update-filter` | The same with the Subscription Filter NextGroupStart, which it may not (§10.2.9) |
| `update-new-group-request` | The same with NEW_GROUP_REQUEST 7, which it may not (§10.2.13) |
| `accept` | The application accepts, with End Location {5, 1}: what was asked for |
| `accept-before-start` | It accepts with End Location {3, 3}, before the Start Location |
| `accept-at-start` | It accepts with End Location {3, 4}, the Start Location |
| `accept-beyond-end` | It accepts with End Location {9, 0}, beyond what was asked for |
| `reject` | It rejects, with `DOES_NOT_EXIST` |

What the rows hold: a REQUEST_UPDATE that comes before the answer gets its REQUEST_OK after the FETCH_OK, since responses carry no Request ID (§10.9); one that comes after a rejection gets none; an End Location in the answer that is before the Start Location is not sent, since its receiver would close the session (§10.13: "If End Location is smaller than the Start Location in the corresponding FETCH the receiver MUST close the session"); one beyond what was asked for is not checked by either session, though §10.13 has the answer end at what was asked for or before.

## Where the bytes come from

Every FETCH is `../moqt-18/fetch-standalone-moqtail-rs.hex` or `../moqt-18/fetch-relative-joining-moqtail-rs.hex`, captured from the `moqtail` crate 0.15.0, with its Request ID, its range, its Fetch Type or its one parameter changed as the row needs. `generate.ts` makes both tables: `bun fixtures/peer-fetch/generate.ts`. The REQUEST_UPDATE messages of `updates.tsv` are written by each test from the step: no capture of one exists.

## Not in the tables

- How many REQUEST_UPDATEs may wait for the answer to a FETCH, and how many FETCHes of the peer may be open at once: `held-fetch-updates`, `peer-fetches` and `peer-requests` of [`fixtures/limits/`](../limits/README.md), and the rows "REQUEST_UPDATE for a FETCH, received" and "Requests of the peer that are open at once" of [the implementation notes](../../docs/moqt-implementation-notes.md).
- What ends a FETCH that was accepted: the streams are not the session's in the Rust implementation.

## `served-end.tsv`

What becomes of the request stream of a FETCH that a publisher served to its end: it sent FETCH_OK, and wrote the fetch stream and finished it. Three implementations read the rows: the relay (for a fetch stream it wrote itself; one it passed on ends the same way), the Rust client with `--answer-fetch`, and `@xmoq/moqt`. The row "The request stream of a FETCH that was served to its end, at a publisher" of [the implementation notes](../../docs/moqt-implementation-notes.md) has the reasons.

One row a line, six fields separated by tabs, no header. Every row ends with one line feed; there are no empty rows.

| Field | Meaning |
|---|---|
| name | What the row is about; unique |
| before | What the subscriber did with its direction of the request stream before the fetch stream was finished: `-` for nothing, `fin` |
| after | What the subscriber does afterwards, in order and separated by commas. `nothing`. `fin`, `reset`: it ends its direction, the reset with `CANCELLED` (`0x1`). `stop-sending`: it sends STOP_SENDING for the publisher's direction, with `CANCELLED`. `update`: it sends a REQUEST_UPDATE with a Request ID it has not used and no parameters. `wait`: it does nothing until the publisher has ended its own direction for want of anything from the subscriber, which the row then says with `wait>fin` |
| own | What the publisher puts on its direction of the request stream, once: `fin` at once, which for a row with `fin` or `reset` in `after` is right after that; `reset:0x1`, a reset with `CANCELLED`, at once; `wait>fin`, a FIN when the time it waits for the subscriber is over, and not before |
| peer | What the publisher does to the subscriber's direction at once: `-` for nothing, `stop:0x1` for STOP_SENDING with `CANCELLED` |
| update | `no`: the REQUEST_UPDATE of the row gets no answer, whichever comes after it, and its Request ID counts as used. `-` in a row without one |

**The times are not in the table**, and differ: how long the publisher waits for the subscriber before its FIN is the same in all three (provisional; `FETCH_PEER_END_WAIT_MS` of `xmoq-relay-core`, `PEER_END_WAIT` of `xmoq-publisher`, `peerStreamEndTimeoutMs` of `@xmoq/moqt`). What follows that FIN for a subscriber that still does not end its direction is not a row either: the relay stops it after `HubConfig::request_close_timeout` more, `@xmoq/moqt` after `peerStreamEndTimeoutMs` more, and the Rust clients (`xmoq-publisher`) stop it with `CANCELLED` at once, as they finish their own. The `peer` field says only what is done at once.

**What a test can see differs too.** A subscriber that stopped the publisher's direction cannot read what the publisher did with it: the relay's test, which is a peer of the relay over an in-memory connection, checks that row by the `peer` field and by what the relay still holds, and not by `own`. The Rust client's test has the stream itself, and sees the reset.

**Not in the table:** a FETCH that was refused, and one whose fetch stream was reset or never came. There the three do not do the same.
