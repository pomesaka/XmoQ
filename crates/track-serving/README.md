# xmoq-track-serving

What a client of the relay needs to serve tracks of its own on a session it also subscribes on. The gatherer of the chat ([`xmoq-chat`](../chat/README.md)) uses it. Such a client is told of namespaces under a prefix, subscribes to a track of each, publishes one track for each prefix with one Object in each Group, and answers the relay's FETCH for it.

| Module | What it has |
|---|---|
| `backoff` | How long to wait before connecting again, and before asking for a `SUBSCRIBE_NAMESPACE` again: 1 s, doubling up to 30 s, and 1 s again after one that lasted a minute (provisional) |
| `fetch` | What a FETCH gets of a track whose Groups have one Object each and no gap: the latest Groups of its range up to a limit, and an End of Unknown Range indicator for those left out (why, at the top of `src/fetch.rs`); the pieces of the answer; the bytes the answers being written may hold together, to a bound that each client sets. No input or output |
| `streams` | The tasks that write the streams, one task for each stream: a request stream the client opens (it reads the answers and writes what it is told), the subgroup stream of one Group, and the answer to a FETCH on its request stream and a fetch stream of its own |

The bounds of `streams` belong to every client that uses it. **All are provisional.**

| What | Bound |
|---|---|
| Messages for one request stream not written yet, the answers to `REQUEST_UPDATE` among them | 64 (`COMMAND_QUEUE`). None is dropped (§10.9: "exactly one"); the client cancels the request with `EXCESSIVE_LOAD` instead |
| A Group's subgroup stream | 1 s for the relay to allow the stream, then 5 s for the relay to take it. After that the stream is reset and the Group is not sent live |
| The relay's direction of a request stream after the client finished its own | 3 s (`PEER_END_WAIT`), then stopped with `CANCELLED` |

The receiving side of a viewer (its SUBSCRIBE, the catalog, the Timestamps of LOC) is not here.
