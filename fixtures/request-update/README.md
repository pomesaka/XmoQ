# What a client does with a REQUEST_UPDATE it receives

Both sessions, the Rust one (`crates/moqt`) and the TypeScript one (`packages/moqt`), read `received.tsv` in their tests: a REQUEST_UPDATE (§10.9) that a client receives, by the request it comes on and the state of that request, and what the session makes of it. The REQUEST_UPDATEs were written by hand from §10.9 and §10.2; the verdicts are the decisions of both implementations, and why each is decided so is at the test that reads the table. The answers a sender of updates receives are in [`fixtures/update-answers/`](../update-answers/README.md), and the updates of a FETCH in [`fixtures/peer-fetch/`](../peer-fetch/README.md).

## Format

`received.tsv`, one row per case, five columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The request the messages come on: `own-publish`, `own-subscribe`, `own-publish-namespace` or `own-track-status`, a request of the client with Request ID 0; or `peer-subscribe`, a SUBSCRIBE of the peer, a server, with Request ID 1 |
| 3 | The state of that request: `pending` (not answered), `established` (answered with REQUEST_OK, SUBSCRIBE_OK, or accepted), `done` (this end sent PUBLISH_DONE after it was established), or `refused` (this end refused it with REQUEST_ERROR) |
| 4 | The messages, one after another, in hexadecimal digits with no spaces. Each REQUEST_UPDATE has a Request ID of the peer's (§10.1) |
| 5 | What the last of them leads to: `ok` (REQUEST_OK, and the subscription goes on), `refused` (REQUEST_ERROR, after which the subscription ends with PUBLISH_DONE `UPDATE_FAILED`), `refused-and-over` (REQUEST_ERROR, and the request is over), `ignored` (no answer, nothing changes), or `closed:<code>` (the session is closed with that Session Termination Error Code, §3.5) |
