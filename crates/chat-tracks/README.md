# xmoq-chat-tracks

The tracks of the chat (decided on 2026-10-24), as every side of it uses them. No input or output, and nothing to build in C: the terminal viewer depends on it without the store of the gatherer ([`xmoq-chat`](../chat/README.md), whose `rusqlite` needs a C compiler).

| Module | What it has |
|---|---|
| `names` | `person_namespace` (`xmoq/chat/<room>/<name>`), `room_namespace` (`xmoq/chat/<room>`), the track names `said` and `lines`, and the participant a namespace is of |
| `payloads` | `Said` with `read_said` and `Said::payload` (`{"text","sentAt"}`); `Line` with `read_line` and `Line::payload` (`{"name","text","sentAt","at"}`); `Malformed`, why a payload is no line |
| `limits` | `Limits` and its defaults: 512 bytes of text a line, 5 lines a second a person, 1,000 lines a FETCH; how far a setting may take them |

What the payloads read as is the shared table of [`fixtures/chat-tracks/`](../../fixtures/chat-tracks/README.md), which the TypeScript reader of [`@xmoq/chat`](../../packages/chat/README.md) reads as well.
