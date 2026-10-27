# xmoq-transcript-tracks

The tracks of the transcripts (decided on 2026-10-25), as every side uses them: the transcriber ([`xmoq-transcriber`](../transcriber/README.md)) writes them, and an agent and a page ([`@xmoq/transcript`](../../packages/transcript/)) read them. It does no input or output and needs nothing built from C. A reader does not need the C++ and cmake that the transcriber's whisper.cpp needs.

| Module | What it has |
|---|---|
| `names` | `transcript_namespace` (`xmoq/transcript/<the fields of the prefix listened to>`), the track name `speech`, and `listened_of`, the prefix a track is the transcript of. Why a namespace of its own and not a track in the broadcast's namespace: at the top of `src/names.rs` |
| `payloads` | `Speech` with `read_speech` and `Speech::payload`; `Malformed`, why a payload is no piece |

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

Every number is an integer from 0 to 2^53 - 1, which a JavaScript number holds exactly. What the payloads read as is the shared table of [`fixtures/transcript-tracks/`](../../fixtures/transcript-tracks/README.md).
