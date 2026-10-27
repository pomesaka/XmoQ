# The answers a subscriber gets to its REQUEST_UPDATEs

A subscriber sends REQUEST_UPDATEs on the stream of its subscription, and the publisher answers each with REQUEST_OK or REQUEST_ERROR, in the order sent (§10.9). After a refusal the publisher ends the subscription with PUBLISH_DONE `UPDATE_FAILED` (§10.9.1). A publisher may coalesce several updates and answer them with "only a single REQUEST_ERROR" (§10.9.1), or answer each. Both sessions read `answers.tsv` in their tests: the Rust session for a SUBSCRIBE and for a subscription a PUBLISH made, `@xmoq/moqt` for a SUBSCRIBE, which is the only subscription it updates. The reasons are at `update_answered` of `crates/moqt/src/session.rs` and `onUpdateAnswered` of `packages/moqt/src/core/requests.ts`.

## `answers.tsv`

One row a line, three fields separated by tabs, no header. Every row ends with one line feed; there are no empty rows. Each row begins with an Established subscription.

| Field | Meaning |
|---|---|
| name | What the row is about; unique |
| steps | The steps, separated by spaces |
| events | For each step, in the same order and separated by ` \| `, what the session makes of it. A row ends at `closed` |

| Step | What happens |
|---|---|
| `update` | The subscriber sends REQUEST_UPDATE with no parameter |
| `ok` | The publisher sends REQUEST_OK with no parameter |
| `error` | The publisher sends REQUEST_ERROR `NOT_SUPPORTED` (`0x3`) |
| `done` | The publisher sends PUBLISH_DONE `UPDATE_FAILED` (`0x8`) |

| Event | Meaning |
|---|---|
| `-` | The update is sent |
| `accepted` | The application learns that the update was accepted |
| `failed` | The application learns that the update was refused |
| `after-refusal` | An answer to an update sent before a refused one: the application learns nothing of it (the Rust session says so with an event of its own, `@xmoq/moqt` with none) |
| `done` | The subscription ends |
| `closed 0x3` | The session is closed with `PROTOCOL_VIOLATION`: an answer that answers no update |

The steps are written as REQUEST_UPDATE, REQUEST_OK, REQUEST_ERROR and PUBLISH_DONE by each test: no capture of these answers exists.
