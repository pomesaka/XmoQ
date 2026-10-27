# The limits of the two sessions

The limits on what an endpoint takes from its peer, and the times it waits, of the Rust session (`xmoq-moqt`, `xmoq-moqt-io`) and the TypeScript one (`@xmoq/moqt`). Where the draft names a value (§2.4.1, §10), it is that value; every other one is **provisional**, and its reason is at the constant. Both test suites read `limits.tsv` and compare each row with their own constant, so that a value changed on one side alone fails a test.

## Format

`limits.tsv`, one row per limit, three columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the limit; unique |
| 2 | The value of the Rust session; `-` where it has none |
| 3 | The value of the TypeScript session; `-` where it has none |

Bytes are bytes, `-ms` is milliseconds, and a count is a count.

## The rows where the two differ

The table shows them; it does not make them the same.

| Row | Why |
|---|---|
| `peer-requests` | The TypeScript session reads 64 request streams of the peer at once, and an accepted subscription keeps its stream open (`maxPeerRequests`, `packages/moqt/src/core/limits.ts`) |
| `subgroup-stream-bytes`, `object-payload-bytes`, `object-properties-bytes` | The Rust session bounds an Object by the bytes one subgroup stream holds undecoded (`StreamConfig`); the TypeScript one bounds its payload and its properties apart |
| `peer-fetches`, `held-fetch-updates` | The rows "Requests of the peer that are open at once" and "REQUEST_UPDATE for a FETCH, received" of [the implementation notes](../../docs/moqt-implementation-notes.md) |
| `ended-by-publish-done` | The Rust session remembers requests it ended with PUBLISH_DONE by a count, for the messages that cross the end; the TypeScript one keeps such a request until the peer ends its stream, at most `peerStreamEndTimeoutMs` (3 s), and forgets it then |
| `held-before-setup` | The row "Objects and fetch streams that come before the peer's SETUP, at a client" of the implementation notes |
