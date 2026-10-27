# What a room answers, and how a client reads it

A room is asked over HTTP, with JSON: where its relay is, tokens for that relay, and the seats of a meeting. What happens in a room as it goes (a broadcast that begins, who is there, reactions, who comes to a meeting and leaves) goes over MoQ, and not through the room. There are two kinds of room, each under a path of its own: the room of a broadcast and the room of a meeting.

The room is [`@xmoq/room`](../../packages/room/README.md). Two clients read its answers, and read `answers.tsv` in their tests: `@xmoq/room-client`, which the pages use, and `xmoq-viewer`, which the terminal viewer and the reader for agents use.

## The requests

| Request | What it asks for | Answers |
|---|---|---|
| `GET /broadcasts/<name>` | The room of a broadcast: its relay, the namespace of its broadcast (`xmoq/broadcast/<name>`, one a room), the prefix of its transcript | 200 |
| `POST /broadcasts/<name>/tokens` with `{role, namespace}` | A token for the relay, `pub` or `sub`, in a namespace of that room: its broadcast, its chat, its transcript | 200, `token` null where the room has no key or makes none for that namespace |
| `GET /meetings/<name>` | The room of a meeting: its relay, the prefix of its participants' namespaces (`xmoq/meet/<name>`), the prefix of its transcript, how many seats it has | 200 |
| `POST /meetings/<name>/tokens` with `{role, namespace}` | As for a broadcast, in a namespace of the meeting's room: its chat, its transcript | 200 |
| `POST /meetings/<name>/seats` with `{participant}` | A seat for the participant: its tokens, the lease it is renewed with, until when it holds, and how often to renew it | 200 a seat; 409 `full` or `refused`; 400 or 404 `refused` |
| `POST /meetings/<name>/seats/<participant>/renew` with `{lease}` | The seat for longer, with new tokens | 200 a seat; 404 `gone` where it has run out or was never there |
| `DELETE /meetings/<name>/seats/<participant>` with `{lease}` | Leaving: the seat is free at once | 204, nothing |

**A seat holds until `expiresAt`** (milliseconds since the Unix epoch, the room's clock) and is renewed every `renewMs`; one that is not renewed is free again when it runs out. Both times are the room's settings, provisional. The `lease` is the seat's own: whoever renews or leaves a seat names it, so that another client cannot take a participant's seat by its id. Why each answer is what it is, at the top of `packages/room/src/worker.ts`.

## `answers.tsv`

One row a line, five columns separated by tabs, no header; every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique |
| 2 | The request the answer is to, as above; for the reader, it says which kind of answer to read the body as |
| 3 | The status of the answer |
| 4 | The body, one object of JSON; `-` for none |
| 5 | What a client reads: one of the forms below, or `malformed` for a body it does not take (a field missing, or of another type) |

| Form | Read from |
|---|---|
| `broadcast relay=<url> cert=<hex> namespace=<a/b> transcript=<a/b>` | The room of a broadcast; `-` for a relay or a transcript that is null |
| `meeting relay=<url> cert=<hex> namespace=<a/b> transcript=<a/b> most=<n>` | The room of a meeting, the same |
| `seat participant=<id> pub=<token> sub=<token> lease=<lease> expires=<ms> renew=<ms>` | A seat, and a seat renewed; `-` for a token that is null |
| `full <most>`, `refused <why>`, `gone`, `left` | The answers that are no seat; `why` is one of `not-a-participant`, `not-a-meeting-room`, `taken` |
| `token <token>` | A token; `-` for null |

A namespace is written with its fields joined by `/`, as the tokens have it.

**What a client does not take** (the stricter of the two readers, as a receiver's checks are safer strict): a status the table of requests above does not give for that request, whatever the body; a relay whose `url` does not begin with `https://` (a relay over WebTransport stands on HTTPS alone) or whose `certSha256Hex` is not 64 hex digits; a namespace or a prefix of a transcript with no field, or with a field that is empty. Each is `malformed`.
