# The chat's tracks: their names and payloads

The JSON a participant writes on `said` and the gatherer writes on `lines` (decided on 2026-10-24), and on `reacted`, `reactions` and `present` (decided on 2026-10-26), and what a reader makes of it. `xmoq-chat-tracks` (`crates/chat-tracks/src/payloads.rs` and `reactions.rs`) and `@xmoq/chat` (`packages/chat/test/payloads.test.ts`) read the tables in their tests, so that the same bytes give the same line, or the same reason why there is none. Written by hand from the decision; no other implementation writes these payloads.

The rows after `not JSON` of `said.tsv`, and after `an array` of `lines.tsv`, were added with the TypeScript reader (2026-10-24): they are where `JSON.parse` and `serde_json` read the same bytes differently (`-0`, `1.0`, `1e400`, an escaped lone surrogate, nesting of 128, a member twice), and were checked by running the Rust test on them before the TypeScript reader was written to them. Bytes a text table cannot hold (not UTF-8, a byte order mark, a control character as it is in a string) are cases of the two tests themselves.

Columns are separated by a tab. A text column holds a JSON string literal, so that quotes, backslashes and line breaks of the text are written as JSON writes them; `-` is "none".

## `said.tsv`

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The payload, as UTF-8 |
| 3 | The `text` read, as a JSON string literal |
| 4 | The `sentAt` read |
| 5 | Why it is no line: `not-an-object`, `text`, `sent-at` |
| 6 | `yes` where writing what was read gives the payload back, byte for byte |

## `lines.tsv`

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The Group ID the Object 0 is of: the number of the line in its room |
| 3 | The payload, as UTF-8 |
| 4 | The `name` read, as a JSON string literal |
| 5 | The `text` read, as a JSON string literal |
| 6 | The `sentAt` read |
| 7 | The `at` read |
| 8 | Why it is no line: `not-an-object`, `name`, `text`, `sent-at`, `at` |
| 9 | `yes` where writing what was read gives the payload back, byte for byte |

## `names.tsv`

What a namespace, or a track, of the chat is read as: the participant whose `said` it is, the room whose `lines` it is, or nobody's. A room is `<kind>/<room>`, the kind `broadcast` or `meeting` (`Room`, `crates/chat-tracks/src/names.rs`). Read by `xmoq-chat-tracks` (`names.rs`), and by `@xmoq/chat` (`test/names.test.ts`), which makes the names of the rows of a participant and of a room's lines and reads none. A field that is not UTF-8 is nobody's, a case of the Rust test itself.

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The fields of the namespace, a JSON array of strings |
| 3 | The track's name, where the row is of a track; `-` where it is of a namespace |
| 4 | What it is read as: `participant <kind>/<room> <name>` for a namespace, `lines <kind>/<room>` for a track, `none` for nobody's |

## `reacted.tsv`, `reactions.tsv` and `present.tsv`

A participant says a reaction on its track `reacted`, one Object a press; the gatherer counts the reactions of a room over a while (`--reaction-batch-ms`, 100 ms, provisional) and publishes them on the room's track `reactions`, one Object a while that had some, and how many participants it follows in the room on `present`, one Object a change. Neither is kept: no FETCH and no store. `present` has the number alone, and no name (decided on 2026-10-26: the participants the gatherer follows, a name twice counted once). An emoji is a string of 1 to 8 UTF-16 units (an emoji with its modifiers); what it shows is not looked at. A count is a whole number, of `reactions` from 1 and of `present` from 0, up to 2^53 - 1. How fast one participant may react, and how many emoji one while may have, are the gatherer's (`--reactions-per-second`, 5; `--reaction-kinds`, 16) and not the reader's.

`reacted.tsv`:

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The payload, as UTF-8 |
| 3 | The `emoji` read, as a JSON string literal |
| 4 | The `sentAt` read |
| 5 | Why it is no reaction: `not-an-object`, `emoji`, `sent-at` |
| 6 | `yes` where writing what was read gives the payload back, byte for byte |

`reactions.tsv`:

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The payload, as UTF-8 |
| 3 | The `counts` read, as a JSON object, its members in the order they were read |
| 4 | The `at` read |
| 5 | Why it is no count of reactions: `not-an-object`, `counts`, `at` |
| 6 | `yes` where writing what was read gives the payload back, byte for byte |

`present.tsv`:

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The payload, as UTF-8 |
| 3 | The `count` read |
| 4 | The `at` read |
| 5 | Why it is no count: `not-an-object`, `count`, `at` |
| 6 | `yes` where writing what was read gives the payload back, byte for byte |
