# xmoq-chat-tracks

The tracks of the chat, as every side of it uses them: the gatherer ([`xmoq-chat`](../chat/README.md)), the terminal viewer ([`xmoq-watch`](../watch/README.md)), the reader for agents ([`xmoq-agent`](../agent/README.md)) and the pages ([`@xmoq/chat`](../../packages/chat/README.md)). No input or output, and nothing to build in C: the terminal viewer depends on it without the store of the gatherer, whose `rusqlite` needs a C compiler.

| Who | Announces | Track | One Group | Payload of its Object 0 |
|---|---|---|---|---|
| A participant | `PUBLISH_NAMESPACE` `xmoq/chat/<kind>/<room>/<name>` | `said` | one line | `{"text": string, "sentAt": number}` |
| A participant | the same | `reacted` | one reaction | `{"emoji": string, "sentAt": number}` |
| The gatherer | PUBLISH (why, at the top of `crates/chat/src/bin/xmoq-chat/session.rs`) | `lines` of `xmoq/chat/<kind>/<room>` | one line; the Group ID is its number in the room | `{"name": string, "text": string, "sentAt": number, "at": number}`, in that order |
| The gatherer | PUBLISH | `reactions` of `xmoq/chat/<kind>/<room>` | what one while (`--reaction-batch-ms`) that had reactions counted; the Group ID is the gatherer's clock | `{"counts": {emoji: number, ...}, "at": number}`, the emoji in the order each came first |
| The gatherer | PUBLISH | `present` of `xmoq/chat/<kind>/<room>` | how many are in the room, at each change; the Group ID is the gatherer's clock | `{"count": number, "at": number}`, no names |

`<kind>` is `broadcast` or `meeting`. `sentAt` is the sender's clock and `at` the gatherer's when it took the line, ended the while or counted the room, all milliseconds since the Unix epoch. `name` is the last field of the sender's namespace. An emoji is 1 to 8 UTF-16 units. The limits of a line and of the reactions, with their defaults, are `src/limits.rs`.

What the names and the payloads read as are the shared tables of [`fixtures/chat-tracks/`](../../fixtures/chat-tracks/README.md), whose tables of the payloads `@xmoq/chat` reads as well.
