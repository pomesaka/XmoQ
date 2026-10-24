# The payloads of the chat's tracks

The JSON a participant writes on `said` and the gatherer writes on `lines` (decided on 2026-10-24), and what a reader makes of it. `xmoq-chat-tracks` (`crates/chat-tracks/src/payloads.rs`) and `@xmoq/chat` (`packages/chat/test/payloads.test.ts`) read both tables in their tests, so that the same bytes give the same line, or the same reason why there is none. Written by hand from the decision; no other implementation writes these payloads.

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
