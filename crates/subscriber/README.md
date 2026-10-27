# xmoq-subscriber

What a client of the relay needs to read the tracks of others on its session. The terminal viewer ([`xmoq-watch`](../watch/README.md)) and the reader for an agent ([`xmoq-agent`](../agent/README.md)) use it, through [`xmoq-viewer-io`](../viewer-io/src/lib.rs) for the chat.

| Module | What it has |
|---|---|
| `kept` | A kept track (`KeptTrack`): subscribed to, asked for again by the rule of [`xmoq-again`](../again/README.md) after an end or a refusal, given up on the session or not after a SUBSCRIBE that could not be sent, and what a malformed track (§2.4.2) ends. What becomes of it after each thing is `decide`, without input or output |
| `follow` | A followed track (`Follower`): a kept track of one numbered item a Group, its past by an Absolute Joining FETCH, closed once it is over, and its live items after it |
| `followed` | What a followed track decides, without input or output: which of its past to fetch (`past_to_fetch`), the items not fetched (`Gap`), and which items are given, each once and in the order of their numbers (`Items`) |
| `catalog` | The catalog of a broadcast that may not be there yet (`FollowedCatalog`), kept as a track is; its Joining FETCH is closed once the catalog was read, with a FIN where it brought the catalog and cancelled where the subscription was first |
| `namespaces` | A `SUBSCRIBE_NAMESPACE` kept on a session (`KeptNamespace`): when it is asked for again, after the role's pause where it ended (`Pause` of `xmoq-again`: 1 s doubling to 30 s for the gatherer and the transcriber, 1 s for the agent) and after its Retry Interval where it was refused. The client sends it and reads its stream itself |
| `session` | The requests and the events of the session they go by: the SUBSCRIBE and its answer, the catalog of one broadcast (`Requests::catalog`), the events read ahead while an answer is waited for (`Events`), SETUP, and the refusal of every request of the peer |

The name of the catalog track and the most a catalog may be are [`xmoq-msf`](../msf/src/lib.rs)'s. What a client of each role values (how many of the latest items of a followed track it fetches, how many it catches up) is the caller's: the chat's are `RECENT_LINES` and `CATCH_UP_LINES` of [`xmoq-viewer`](../viewer/src/lib.rs).

What becomes of a kept track after each thing that can happen to it is the shared table of [`fixtures/kept-track/`](../../fixtures/kept-track/README.md), whose rows of no limit on the times and of a required track `@xmoq/tracks` reads as well; which of its past a followed track fetches is that of [`fixtures/chat-history/`](../../fixtures/chat-history/README.md), which `@xmoq/tracks` reads.
