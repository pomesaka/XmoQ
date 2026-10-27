# What a client refuses a request of the peer with

A client that publishes receives requests on bidirectional streams the relay opened. Of those it does not serve, it answers each with REQUEST_ERROR `NOT_SUPPORTED` (`0x3`), whether it serves SUBSCRIBE or not (§4). Why, and why not `UNINTERESTED`, is at `refusal_code` of the Rust client (`crates/moqt-client/src/main.rs`). The Rust client and `@xmoq/moqt` read `refusals.tsv` in their tests.

## `refusals.tsv`

One row a line, three fields separated by tabs, no header. Every row ends with one line feed; there are no empty rows.

| Field | Meaning |
|---|---|
| request | `subscribe`, `publish`, `publish-namespace`, `fetch` (Standalone), `subscribe-namespace`, `track-status`, or `subscribe-tracks` (refused by the session, before the client sees it) |
| serves | `yes`: the client serves SUBSCRIBE (the Rust client in the mode that answers one for its track; `MoqtConnection.serve` with `subscribe` in `@xmoq/moqt`). `no`: it serves nothing. The rows of both pin that serving SUBSCRIBE changes none of them |
| code | The Error Code of the REQUEST_ERROR, in hex |

Not in the table:

- A SUBSCRIBE where the client serves SUBSCRIBE: the application answers it. The Rust client refuses one for a track it did not announce with `DOES_NOT_EXIST`.
- A FETCH the client answers (`--answer-fetch` of the Rust client, `ServeHandlers.fetch`), and the codes it refuses one with: `fixtures/peer-fetch/`.
- What the session refuses by itself before the application sees the request (`DUPLICATE_SUBSCRIPTION`, `EXCESSIVE_LOAD`): `fixtures/peer-subscribe/`.
