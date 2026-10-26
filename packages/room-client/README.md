# @xmoq/room-client

Asking a room ([`@xmoq/room`](../room/README.md)) over HTTP: where its relay is, tokens for that relay, and the seats of a meeting. What the room answers, and how this reads it, is [`fixtures/room-api/`](../../fixtures/room-api/README.md), which the Rust client (`crates/viewer/src/room.rs`) reads as well.

| Export | What it does |
|---|---|
| `broadcastRoom(room)`, `meetingRoom(room)` | `GET` the room of a broadcast or of a meeting: its relay, its namespace, the prefix of its transcript, and of a meeting how many seats it has |
| `tokenOfARoom(kind, room, role, namespace)` | A token of that room, `pub` or `sub`; null where it makes none |
| `takeSeat(room, participant)` | A seat, or why there is none (`full`, `refused` with why) |
| `keepSeat(room, seat)` | Renews the seat every `renewMs` the room said, with its lease; `seat` has the newest tokens, `lost` says when a renewal failed or the seat was gone, `leave()` stops and frees it |
| `renewSeat`, `leaveSeat` | The two requests `keepSeat` makes |
| `readBroadcastRoom`, `readMeetingRoom`, `readSeatAnswer`, `readToken` | The readers of the answers, in `src/core`, which does no input or output |

A body this client does not take rejects with `RoomAnswerError`, with the status. It is in layer 0 (`scripts/ci/check-layers.sh`): it needs no other package, and the room and the pages that ask it are above it.
