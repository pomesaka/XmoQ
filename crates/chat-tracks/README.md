# xmoq-chat-tracks

The tracks of the chat (decided on 2026-10-24), as every side of it uses them. No input or output, and nothing to build in C: the terminal viewer depends on it without the store of the gatherer ([`xmoq-chat`](../chat/README.md), whose `rusqlite` needs a C compiler).

| Module | What it has |
|---|---|
| `names` | `Room`, a kind (`broadcast` or `meeting`) and a name, read from and written as `<kind>/<room>`; `person_namespace` (`xmoq/chat/<kind>/<room>/<name>`), `room_namespace` (`xmoq/chat/<kind>/<room>`), the tracks of a participant (`PersonTrack`: `said`, `reacted`) and of a room (`RoomTrack`: `lines`, `reactions`, `present`) with `person_track` and `room_track`, the participant a namespace is of, and which of them a track is (`track_of`) |
| `payloads` | `Said` with `read_said` and `Said::payload` (`{"text","sentAt"}`); `Line` with `read_line` and `Line::payload` (`{"name","text","sentAt","at"}`); `Malformed`, why a payload is no line |
| `reactions` | `Reacted` with `read_reacted` and `Reacted::payload` (`{"emoji","sentAt"}`); `Reactions` with `read_reactions` and `Reactions::payload` (`{"counts","at"}`, the counts in their order); `Present` with `read_present` and `Present::payload` (`{"count","at"}`); an emoji is 1 to 8 UTF-16 units (`is_emoji`) |
| `limits` | `Limits` and its defaults: 512 bytes of text a line, 5 lines a second a person, 1,000 lines a FETCH; the reactions counted over 100 ms, 5 a second a person, 16 emoji a while (provisional); how far a setting may take them |

What the names and the payloads read as are the shared tables of [`fixtures/chat-tracks/`](../../fixtures/chat-tracks/README.md); the TypeScript reader of [`@xmoq/chat`](../../packages/chat/README.md) reads those of the payloads as well.
