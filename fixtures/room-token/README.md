# The token of a room

The text of a token the room makes and the relay reads, `v1|<namespace>|<pub or sub>|<expiry>|<mac>`, the mac HMAC-SHA-256 with the key they share over the text before the last `|`.

| File | Read by |
|---|---|
| `tokens.tsv` | [`xmoq-room-token`](../../crates/room-token/src/lib.rs) (`RoomKey::make` and `RoomKey::read`) and `@xmoq/room` (`tokenText`, `packages/room/src/token.ts`) |

No header, columns separated by tabs, every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique in the file |
| 2 | The key, in hexadecimal |
| 3 | The namespace, its fields joined with `/` |
| 4 | The role, `pub` or `sub` |
| 5 | The expiry, seconds since 1970 |
| 6 | The token |

The tokens were made with Python's `hmac` module (Python 3, `hmac.new(key, signed, hashlib.sha256).hexdigest()`) on 2026-10-26, not by either reader. The first row is the value `xmoq-room-token` pinned before this table.
