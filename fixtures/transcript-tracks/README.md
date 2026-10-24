# The payload of a transcript's track

The JSON the transcriber writes on `speech` (decided on 2026-10-25), and what a reader makes of it. `xmoq-transcript-tracks` (`crates/transcript-tracks/src/payloads.rs`) reads the table in its tests. A TypeScript reader is to read the same table once there is one. The rows were written by hand from the decision; no other implementation writes these payloads. Bytes a text table cannot hold (not UTF-8) are a case of the Rust test itself.

Columns are separated by a tab. A text column holds a JSON literal (`from` an array of strings, `text` a string), so that quotes, backslashes and line breaks are written as JSON writes them; `-` is "none".

## `speech.tsv`

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The Group ID the Object 0 is of: the number of the piece |
| 3 | The payload, as UTF-8 |
| 4 | The `from` read, as a JSON array |
| 5 | The `text` read, as a JSON string literal |
| 6 | The `startUs` read |
| 7 | The `endUs` read |
| 8 | The `at` read |
| 9 | Why it is no piece: `not-an-object`, `from`, `text`, `start-us`, `end-us`, `at` |
| 10 | `yes` where writing what was read gives the payload back, byte for byte |
