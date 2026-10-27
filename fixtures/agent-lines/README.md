# The lines an agent reads and writes

The files of `events/` that a reader writes for an agent, the lines in them, and how a line the agent writes back is read: [`xmoq-agent`](../../crates/agent/README.md) (its `--out`, its stdin), and the meeting's page through a folder (`@xmoq/meeting`). An agent reads the two in the same way, so the two write the same files and lines for the same events, and give it the same guide; both read `lines.tsv`, `event-files.tsv`, `inbox.tsv`, `inbox-reads.tsv`, `AGENT.md` and `call.tsv` in their tests.

## The folder

| Name | What it is |
|---|---|
| `AGENT.md` | The guide for an agent, the text of `AGENT.md` here, put there anew each time the writing begins |
| `events/<number>.jsonl` | The lines of one write, about a second of them, in the order they were written. Never changed after it is whole. `<number>` is the writer's clock in milliseconds since the Unix epoch when it wrote the file, in 13 digits at the least (padded with `0`), or one more than the newest number there where the clock is not after it (`event-files.tsv`); so the names sort as the files were written. A file that is there and empty is being written: the page makes the file before it moves its lines into it. The lines of the folder are the files' lines, in the order of their names |
| `inbox.txt` | What the agent adds lines to (`>>`); read again every second. What is in it when the reader begins is passed over |
| `frames/<when the writing began, ms>/` | The pictures `frame` lines name |

A name of `events/` that is not digits and `.jsonl` is none of the reader's: `xmoq-agent` writes a file as `.<name>.tmp` and moves it to its name.

## The lines

Every line has `type` and `at`, the writer's clock in milliseconds since the Unix epoch when it wrote the line. The keys are in the order of their names. The tables say what `xmoq-agent` does, with its flags; the page of a meeting writes and reads the same lines.

| `type` | Fields | When |
|---|---|---|
| `reader` | `state` (`began`, `ended`); with `began`, `room` (the chat's room, `broadcast/<name>` or `meeting/<name>`) and, from a page, `id` (the page's id in the meeting) | `began`: the first line of each run (`xmoq-agent` started, a page called); `ended`: the last, where the reader stops of itself (`xmoq-agent` on a signal or `--seconds`, a page's stop). None where the process is killed or the page closed |
| `broadcast` | `state` (`began`, `ended`), `namespace` | `began`: the broadcast's catalog came on the session, so the broadcast is there; `ended`: after `began`, the catalog's subscription ended (the broadcast left) or the session ended. Each `began` has its `ended`; a session in which the broadcast never came writes neither. With `--url`, the namespace of `--namespace`. Not in a meeting |
| `chat` | `number`, `from`, `text`, `sent_at` (the sender's clock), `gathered_at` (the gatherer's clock); `by` (`agent`) on a line the agent said | A line of the room, each once, in the order of its number: the latest `RECENT_LINES` (`xmoq-viewer`) when the reader comes, then each as it is said. The reader's own lines come back as well, with `by`: `xmoq-agent` marks every line of its own name, and a page the lines it said for the agent this time it was called, known by the page's name, `sent_at` and `text` |
| `chat_gap` | `after`, `before` | More lines were said while the reader was away than `--chat-catch-up-lines`: those between were not fetched |
| `joined`, `left` | `name` | Somebody's chat namespace `xmoq/chat/<kind>/<room>/<name>` was announced, or withdrawn |
| `participant` | `state` (`joined`, `left`), `id` | In a meeting: a participant's broadcast `xmoq/meet/<room>/<id>` was announced, or withdrawn; on a new session, those not told of again within `SETTLE_MS` are written as left. `id` is not the participant's name in the chat |
| `frame` | `path` (from the folder the line is in, with `/`: `frames/<when the run began, ms>/<file>`), `group`, `object`, `timestamp_us` (the Timestamp of LOC of the frame), `width`, `height`, `asked` (`command`, `every`); in a meeting, `id`, whose picture it is | A picture of the broadcast, written as a PNG in `<dir>/frames/<when the run began, ms>/`: asked for with `/frame`, or every `--frame-every-ms`. It is of the latest frame that decoded, which can be older than the moment it was asked for. `/frame` that gets none within `FRAME_WAIT` gets an `error` |
| `speech` | `number` (in the transcript), `from` (the namespace of the broadcast whose voice it was), `text`, `start_us`, `end_us` (on the clock of the Timestamps of LOC of that broadcast's audio), `read_at` (the transcriber's clock, ms) | Something said in a broadcast, as the transcriber (`xmoq-transcriber`) read it: the pieces of `speech` of `xmoq/transcript/<prefix>` ([`fixtures/transcript-tracks/`](../transcript-tracks/README.md)), each once, in the order of its number. The latest `RECENT_LINES` when the reader comes, by a Joining FETCH, then each as it is read; those missed while away, as for the chat. Pieces not fetched are told as an `error`. With `--transcript`, the pieces of every broadcast under that prefix, each with its `from` |
| `error` | `message` | Something went wrong, or a line the agent wrote was not said; the reader goes on |

```json
{"at":1792883451092,"name":"p","type":"joined"}
{"at":1792883451113,"from":"p","gathered_at":1792883451111,"number":0,"sent_at":1792883451110,"text":"hello from p","type":"chat"}
{"asked":"command","at":1792884905154,"group":1792884901959,"height":360,"object":12,"path":"frames/1792884900120/000001-g1792884901959-o12.png","timestamp_us":1792884904928000,"type":"frame","width":640}
{"at":1792883451182,"message":"no command /nothing: /frame is one, and // begins a line that is said","type":"error"}
{"at":1792883451205,"name":"p","type":"left"}
```

The text is as the sender wrote it: a line of JSON escapes what a terminal would act on, and nothing is taken out.

## What a line the agent writes does

A line of stdin of `xmoq-agent`, or one added to `inbox.txt` of either.

| The line | What happens |
|---|---|
| `hello` | Said in the chat. Over `TEXT_BYTES` of `xmoq-chat-tracks`, or while `SAID_WAITING` lines wait for the gatherer, it is not, and an `error` says so |
| `/frame` | A picture of the broadcast now, as a `frame` line; in a meeting, `/frame <id>` names whose, and `/frame` alone is of the participant whose voice was read last. Asked while the reader connects (again), it is kept and asked of the next session's video; within `FRAME_WAIT` from when it was typed, or an `error`. Before the broadcast's catalog came, an `error` (`frame: no broadcast yet`) |
| `//hello` | `/hello` is said: one `/` is taken off, so that a line can begin with `/` |
| `/other` | An `error`: no such command |
| An empty line | Nothing |
| Over `MOST_A_LINE` bytes (`xmoq-agent`) or `MAX_INBOX_LINE_BYTES` (`@xmoq/meeting`) | Not read, and an `error` says so |

## `lines.tsv`

One line a row, two columns separated by a tab, no header; every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique. Each reader's test has its own event for each name |
| 2 | The line, as it is written: one object of JSON, its keys in the order of their names, `at` the writer's clock in milliseconds since the Unix epoch |

The rows pin what is not written down elsewhere: the order of the keys, how a string is escaped (what a terminal would act on, and a line break, as `\u001b` and `\n`; other characters as they are), numbers without a fraction or an exponent, `id` on a `frame` of a meeting and on the first line of a page, `by` on a line the agent said, and the `path` of a `frame` from the folder the line is in (`frames/<when the writing began, ms>/<six digits>-g<group>-o<object>.png`).

## `event-files.tsv`

The name of the next file of `events/`. Four columns separated by tabs, no header; every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique |
| 2 | The names in `events/` before, separated by `,` (`-` for none), those that are no file of events among them |
| 3 | The writer's clock, ms |
| 4 | The name of the file written |

## `inbox.tsv`

What a line an agent writes asks for: a line of stdin of `xmoq-agent`, and a line of `inbox.txt` of a page's folder. Three columns separated by tabs, no header; every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique |
| 2 | The line, without its line feed, as a string of JSON (so that a carriage return and a tab can be in it) |
| 3 | What it asks for: `say <text>`, `frame <id>` (`frame -` for none named), `unknown <name>`, or `nothing`. What follows the first space is as it is, spaces and all |

## `inbox-reads.tsv`

What a reader reads of `inbox.txt` as it is now, after the read before. Six columns separated by tabs, no header; every row ends with one line feed. Made once with a script, so that a long line is not counted by hand.

| Column | Content |
|---|---|
| 1 | The name of the row, unique |
| 2 | The file as it is now, as a string of JSON |
| 3 | The byte where the read before ended |
| 4 | The first bytes of the file at the read before, as a string of JSON |
| 5 | The lines read, as a list of JSON; `null` for one over the limit, which is not read |
| 6 | The byte where the next read begins |

## `AGENT.md`

The text each reader puts in its folder as `AGENT.md`, as it is. The loop in it was run in dash, bash and zsh of macOS on 2026-10-27, with an empty file being written among the files, a name `.<number>.jsonl.tmp`, one `<number>.jsonl.crswap` and one `notes.jsonl`. Its form with `jq` was run in dash and zsh with jq 1.7.1 and with jaq 1.4.0 on 2026-10-27, on the lines of every `type`, alike.

## `call.tsv`

The line a person gives an agent to have it take part, with the folder it names. Three columns separated by tabs, no header; every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique |
| 2 | The folder, as the reader names it: the page with `~/Documents/xmoq/<room>/<id>`, `xmoq-agent` with `--out` in full |
| 3 | The line |
