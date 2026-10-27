# What becomes of a SUBSCRIBE the peer sends

A client receives a SUBSCRIBE on a bidirectional stream the server opened: a relay asks the publisher of a namespace for a track. The session of each implementation decides, before the application sees the request, whether it is handed on, refused, or a reason to close the session. Both implementations read `opened.tsv` in their tests, so that the same bytes in the same state get the same answer from both.

The draft has the rules. §5.1: "an endpoint can have at most one subscription to a Track acting as the publisher and at most one acting as a subscriber. If an endpoint receives a message attempting to establish a second subscription to a Track with the same role, it MUST fail that request with a DUPLICATE_SUBSCRIPTION error" (`0x19`, §10.6.2), and the same for a SUBSCRIBE while one for the Track is in the Pending (publisher) state. §10.1: a Request ID of the wrong parity, or a duplicate, closes the session with `INVALID_REQUEST_ID`. What the draft leaves open, and the two implementations settled, is in [the implementation notes](../../docs/moqt-implementation-notes.md).

## The table

`opened.tsv` has one row a line, four fields separated by tabs:

| Field | Meaning |
|---|---|
| name | What the row is about |
| before | What happened in the session before the row's bytes, below |
| sent | The bytes the server sends on a new bidirectional stream, in hex: one SUBSCRIBE, whole |
| outcome | `handed`: the application gets the request. `refused 0x19`: the session answers REQUEST_ERROR with that code itself, and the application sees nothing. `closed 0x4`: the session is closed with that Session Termination Error Code (§3.5) |

The session is that of a client, after its own SETUP.

| before | What happened |
|---|---|
| `nothing` | Nothing |
| `publish` | This endpoint sent PUBLISH for the track `("xmoq", "phase0")` `"object"`, with Track Alias 1; it has no answer |
| `subscribe` | The server sent the SUBSCRIBE of `sent` with Request ID 1 for the track `"object"`, on another stream; it was handed to the application, which has not answered |
| `subscribe-accepted` | As `subscribe`, and the application accepted it (SUBSCRIBE_OK) |
| `subscribe-done` | As `subscribe-accepted`, and this endpoint ended the subscription with PUBLISH_DONE (`TRACK_ENDED`, `0x2`); the server has not ended its direction of that stream |
| `subscribe-rejected` | As `subscribe`, and the application rejected it (REQUEST_ERROR `DOES_NOT_EXIST`, `0x10`); the server has not ended its direction of that stream |
| `subscribe-rejected-then-update` | As `subscribe-rejected`, and the server then sent, on the stream of that SUBSCRIBE, a REQUEST_UPDATE with Request ID 3 and no parameters (`../moqt-18-by-hand/request-update-request-id-3.hex`) |
| `subscribe-refused-then-update` | As `subscribe`, and the server then sent a second SUBSCRIBE for the track `"object"`, with Request ID 3, on a stream of its own, which the session refused by itself (`DUPLICATE_SUBSCRIPTION`); and after it, on that same stream, a REQUEST_UPDATE with Request ID 5 and no parameters (`../moqt-18-by-hand/request-update-request-id-5.hex`) |

The last two states are about what still comes on the stream of a request that was refused: a REQUEST_UPDATE there gets no answer and uses up its Request ID (§10.1: "Each ... REQUEST_UPDATE ... message consumes a Request ID", with no condition), whether the application refused the request or the session did. The two REQUEST_UPDATE messages are written by hand: no capture of one exists.

## Where the bytes come from

Every `sent` is `../moqt-18/subscribe-moqtail-rs.hex`, captured from the `moqtail` crate 0.15.0, with two changes: its Request ID, which is 0 there and has to be one of a server here (1, 3 or 5; 2 for the row of the wrong parity), and for the rows of another track the last byte of its Track Name (`object` to `objecu`). `generate.ts` makes the table: `bun fixtures/peer-subscribe/generate.ts`.

## Not in the table

- How many requests of the peer may be open at once, and `EXCESSIVE_LOAD` (`0x9`) for one more: `peer-requests` of [`fixtures/limits/`](../limits/README.md), which differs between the two, and the row "Requests of the peer that are open at once" of the implementation notes.
- A SUBSCRIBE that does not decode.
