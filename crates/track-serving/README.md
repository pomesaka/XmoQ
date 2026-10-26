# xmoq-track-serving

What a client of the relay needs to serve tracks of its own on a session it also subscribes on. The gatherer of the chat ([`xmoq-chat`](../chat/README.md)) and the transcriber ([`xmoq-transcriber`](../transcriber/README.md)) use it. Such a client is told of namespaces under a prefix, subscribes to a track of each, publishes one track for each prefix with one Object in each Group, and answers the relay's FETCH for it.

| Module | What it has |
|---|---|
| `answers` | The FETCH requests of the peer that are answered, from the `FETCH_OK` to the end of both directions of the request stream, a task for each stream: how many at once (32, `MAX_ANSWERED_AT_ONCE`), the bytes their answers hold, the answer to a `REQUEST_UPDATE`, a `STOP_SENDING` or a reset of the subscriber on either stream (both streams reset, §5.2), and when the subscriber's direction is stopped; what the session is told (`conclude`). The Rust client answers by it as well, at the pace of its flags (`Pace`). Generic over `Connection`, and tested on `MemoryConnection` |
| `publication` | The PUBLISH of a track, from the PUBLISH to its `PUBLISH_DONE`: what waits for the `PUBLISH_OK`, the streams it opened, and the pause before a PUBLISH that failed is sent again (5 s, `PUBLISH_AGAIN_AFTER`). No input or output; the count of the streams being sent is a `tokio` watch, shared with their tasks |
| `requests` | The requests the relay opens: a FETCH is handed to the client, which plans its answer and hands it back to be written; every other request is refused with `NOT_SUPPORTED`; the steps of `answers` are carried out. Writes the answers, the refusals and the log. Generic over the streams of any `Connection`, and tested on `MemoryConnection` |
| `fetch` | What a FETCH gets of a track whose Groups have one Object each and no gap: the latest Groups of its range up to a limit, of those the publisher still keeps, and an End of Unknown Range indicator for those left out or let go of (why, at the top of `src/fetch.rs`; the Rust client plans by it too, and both read `fixtures/published-fetch/`); the pieces of the answer; the bytes the answers being written may hold together, to a bound that each client sets. No input or output |
| `streams` | The tasks that write the streams, one task for each stream: a request stream the client opens (it reads the answers and writes what it is told), and the subgroup stream of one Group. Generic over `Connection`, and tested on `MemoryConnection` |

The bounds of `streams` belong to every client that uses it. **All are provisional.**

| What | Bound |
|---|---|
| Messages for one request stream not written yet, the answers to `REQUEST_UPDATE` among them | 64 (`COMMAND_QUEUE`), also for a FETCH the gatherer and the transcriber answer (16 in the Rust client, `SAY_QUEUE_OF_A_PAGE`). None is dropped (§10.9: "exactly one"); the client cancels the request with `EXCESSIVE_LOAD` instead |
| A Group's subgroup stream | 1 s for the relay to allow the stream, then 5 s for the relay to take it. After that the stream is reset and the Group is not sent live |
| The relay's direction of a request stream after the client finished its own, and of a FETCH's after its fetch stream was written to its end | 3 s (`PEER_END_WAIT`), then stopped with `CANCELLED` (for a FETCH, as the client finishes its own); no more than 64 FETCH requests wait so, and one more has it stopped at once |

The receiving side of a viewer (its SUBSCRIBE, the catalog, the Timestamps of LOC) is not here.
