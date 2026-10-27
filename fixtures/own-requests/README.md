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
