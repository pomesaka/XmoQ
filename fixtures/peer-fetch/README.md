# What becomes of a FETCH the peer sends

A publisher receives a FETCH on a bidirectional stream the peer opened: a relay asks it for a range of a track. The session of each implementation decides, before the application sees the request, whether it is handed on, refused, or a reason to close the session; and once the application has the request, what is sent for each thing that happens to it. Both implementations read the two tables here in their tests, so that the same bytes in the same state get the same answer from both.

**Every outcome here was written after the row had been run through the Rust session** (`crates/moqt` of `main` at `59d76ab8`, in a test that is not kept), and then through `@xmoq/moqt`. Two outcomes had first been written from reading the code, and were wrong: an End Location that names the same Location as the Start Location is taken (§10.12.3 asks for "the same or a larger Location"), in the FETCH and in the answer.

## `opened.tsv`

One row a line, five fields separated by tabs:

| Field | Meaning |
|---|---|
| name | What the row is about |
| before | What happened in the session before the row's bytes: as in [`../peer-subscribe/README.md`](../peer-subscribe/README.md) |
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
| `accept` | The application accepts, with End Location {5, 1}: what was asked for |
| `accept-before-start` | It accepts with End Location {3, 3}, before the Start Location |
| `accept-beyond-end` | It accepts with End Location {9, 0}, beyond what was asked for |
| `reject` | It rejects, with `DOES_NOT_EXIST` |

What the rows hold: a REQUEST_UPDATE that comes before the answer gets its REQUEST_OK after the FETCH_OK, since responses carry no Request ID (§10.9); one that comes after a rejection gets none; an End Location in the answer that is before the Start Location is not sent, since its receiver would close the session (§10.13: "If End Location is smaller than the Start Location in the corresponding FETCH the receiver MUST close the session"); one beyond what was asked for is not checked by either session, though §10.13 has the answer end at what was asked for or before.

## Where the bytes come from

Every FETCH is `../moqt-18/fetch-standalone-moqtail-rs.hex` or `../moqt-18/fetch-relative-joining-moqtail-rs.hex`, captured from the `moqtail` crate 0.15.0, with its Request ID, its range, its Fetch Type or its one parameter changed as the row needs. `generate.ts` makes both tables: `bun fixtures/peer-fetch/generate.ts`. The REQUEST_UPDATE messages of `updates.tsv` are written by each test from the step: no capture of one exists.

## Not in the tables

- How many REQUEST_UPDATEs may wait for the answer to a FETCH: 64, and one more refuses the FETCH with `EXCESSIVE_LOAD`. The Rust session counts them and leaves the limit to its caller; the relay and `@xmoq/moqt` have it, with a test each.
- How many FETCHes of the peer may be open at once, and `EXCESSIVE_LOAD` for one more: 32 in `@xmoq/moqt` (`limits.maxPeerFetches`); the Rust session counts every request of the peer together, 256.
- What ends a FETCH that was accepted: the streams are not the session's in the Rust implementation.
