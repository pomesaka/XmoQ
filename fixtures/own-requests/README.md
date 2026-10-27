# What comes from the peer for a request of this end

The counterpart of [`fixtures/peer-fetch/`](../peer-fetch/README.md) and [`fixtures/peer-subscribe/`](../peer-subscribe/README.md), which have the requests of the peer. Both sessions, the Rust one (`crates/moqt`) and the TypeScript one (`packages/moqt`), read `events.tsv` in their tests: what comes from the peer for a request of a client, on the request's stream or as a fetch stream, and what the session makes of it. The bytes were written by hand from §10.13 and §10.12.3; the verdicts are the decisions of both implementations, and why each is decided so is at the test that reads the table.

## Format

`events.tsv`, one row per case, five columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The request of the client, Request ID 0, its first: `fetch`, a Standalone FETCH of {4, 2} to {9, 0}; `cancelled-fetch`, that FETCH cancelled by the client; or `subscribe`, a SUBSCRIBE not answered yet |
| 3 | Where the bytes come: `request-stream`, the stream of the request; `fetch-stream`, a unidirectional stream; `two-fetch-streams`, a second unidirectional stream after one with the same bytes |
| 4 | The bytes, in hexadecimal digits with no spaces: a control message on the request stream, or the FETCH_HEADER of a fetch stream |
| 5 | What they lead to: `taken`; `over` (the request is over: the client cancels it); `stopped` (the fetch stream is stopped, the session stays); or `closed:<code>` (the session is closed with that Session Termination Error Code, §3.5) |

`stream-ends.tsv`, one row per case, five columns separated by tabs, no header. Every row ends with one line feed. The TypeScript session reads every row. In Rust the session does not decide what the end of the stream of a request of this end does; the caller of each role does, and reads the rows of its requests: `xmoq-subscriber` those of `subscribe` and `subscribe-namespace`, `xmoq-moqt-client` those of `fetch` and `publish-namespace`, and `xmoq-publisher` those of `publish`, which the gatherer of the chat and the transcriber go by.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The request of the client, Request ID 0, its first: `subscribe`, `fetch` (a Standalone FETCH), `publish`, `publish-namespace` or `subscribe-namespace` |
| 3 | Its state: `pending` (not answered), `established` (answered with SUBSCRIBE_OK, FETCH_OK, PUBLISH_OK or REQUEST_OK), or `done` (PUBLISH_DONE was received for a `subscribe`, or sent for a `publish`) |
| 4 | What the peer does to the stream: `fin` or `reset` of its direction, or `stop-sending` for the client's direction |
| 5 | What it leads to: `ends` (the request is over), `nothing` (it goes on, or stays over), or `reported` (the request was over already, and the end of the peer's direction is reported) |
