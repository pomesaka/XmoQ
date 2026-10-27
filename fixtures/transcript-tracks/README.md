# The payload of a transcript's track

The names of a transcript's track, and the JSON the transcriber writes on `speech`, and what a reader makes of it. `xmoq-transcript-tracks` and `@xmoq/transcript` read the table in their tests, so that the same bytes give the same piece, or the same reason why there is none. The rows were written by hand from the decision; no other implementation writes these payloads. Bytes a text table cannot hold (not UTF-8) are a case of the Rust test itself.

Columns are separated by a tab. A text column holds a JSON literal (`from` an array of strings, `text` a string), so that quotes, backslashes and line breaks are written as JSON writes them; `-` is "none".

## The payload

One piece of speech is one Group of `speech`, and its Object 0 is

```json
{"from":["xmoq","demo"],"text":"Hello everyone.","startUs":1761350400000000,"endUs":1761350402500000,"at":1761350403700}
```

with the members in that order. The Group IDs of a transcript go up by one, without a gap.

| Member | What it is |
|---|---|
| `from` | The namespace of the broadcast whose voice it was, every field |
| `text` | The words whisper.cpp read |
| `startUs`, `endUs` | Where the piece begins and ends, in microseconds, on the clock of the Timestamps of LOC of that broadcast's audio track. A piece keeps up to 200 ms of quiet before its first word and after its last |
| `at` | The transcriber's clock when it had read the piece, in milliseconds since the Unix epoch |

Every number is an integer from 0 to 2^53 - 1, which a JavaScript number holds exactly.

## `names.tsv`

Which track is the transcript of which prefix: `xmoq-transcript-tracks` reads each track and makes the track of each prefix, and `@xmoq/transcript` makes the track of each prefix and reads none.

| Column | What it is |
|---|---|
| 1 | The case |
| 2 | The fields of the namespace, a JSON array of strings |
| 3 | The track's name |
| 4 | The prefix it is the transcript of, a JSON array of strings; `none` for a track that is no transcript |

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
