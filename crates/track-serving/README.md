# xmoq-track-serving

What a client of the relay needs to serve tracks of its own on a session it also subscribes on. The gatherer of the chat ([`xmoq-chat`](../chat/README.md)) and the transcriber ([`xmoq-transcriber`](../transcriber/README.md)) use it. Such a client is told of namespaces under a prefix, subscribes to a track of each, publishes one track for each prefix with one Object in each Group, and answers the relay's FETCH for it.

| Module | What it has |
|---|---|
| `answers` | The FETCH requests of the relay that are answered, from the answer to the end of both directions of the request stream: how many at once (32, `FETCHES_AT_ONCE`), the bytes their answers hold, the answer to a `REQUEST_UPDATE`, and when the relay's direction is stopped. What is to be done to the session and the streams comes back as steps. No input or output |
| `publication` | The PUBLISH of a track, from the PUBLISH to its `PUBLISH_DONE`: what waits for the `PUBLISH_OK`, the streams it opened, and the pause before a PUBLISH that failed is sent again (5 s, `PUBLISH_AGAIN_AFTER`). No input or output; the count of the streams being sent is a `tokio` watch, shared with their tasks |
| `requests` | The requests the relay opens: a FETCH is handed to the client, which plans its answer and hands it back to be written; every other request is refused with `NOT_SUPPORTED`; the steps of `answers` are carried out. Writes the answers, the refusals and the log |
| `backoff` | How long to wait before connecting again, and before asking for a `SUBSCRIBE_NAMESPACE` again: 1 s, doubling up to 30 s, and 1 s again after one that lasted a minute (provisional). Where the next session goes, and the try at once after a GOAWAY, by `NextSession` of [`xmoq-again`](../again/README.md) (`NextTry`) |
| `fetch` | What a FETCH gets of a track whose Groups have one Object each and no gap: the latest Groups of its range up to a limit, of those the publisher still keeps, and an End of Unknown Range indicator for those left out or let go of (why, at the top of `src/fetch.rs`; the Rust client plans by it too, and both read `fixtures/published-fetch/`); the pieces of the answer; the bytes the answers being written may hold together, to a bound that each client sets. No input or output |
| `streams` | The tasks that write the streams, one task for each stream: a request stream the client opens (it reads the answers and writes what it is told), the subgroup stream of one Group, and the answer to a FETCH on its request stream and a fetch stream of its own |

The bounds of `streams` belong to every client that uses it. **All are provisional.**

| What | Bound |
|---|---|
| Messages for one request stream not written yet, the answers to `REQUEST_UPDATE` among them | 64 (`COMMAND_QUEUE`). None is dropped (§10.9: "exactly one"); the client cancels the request with `EXCESSIVE_LOAD` instead |
| A Group's subgroup stream | 1 s for the relay to allow the stream, then 5 s for the relay to take it. After that the stream is reset and the Group is not sent live |
| The relay's direction of a request stream after the client finished its own | 3 s (`PEER_END_WAIT`), then stopped with `CANCELLED` |

The receiving side of a viewer (its SUBSCRIBE, the catalog, the Timestamps of LOC) is not here.
