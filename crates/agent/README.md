# xmoq-agent

A reader of a room for an agent. It joins a room as a participant of its chat, and writes what happens there as lines of JSON, one event a line, on stdout and at the end of `<dir>/events.jsonl`. A line of its stdin is said in the chat under its name.

It connects the way `xmoq-watch` does (`xmoq-viewer` and `xmoq-viewer-io` are the parts the two share): the room says where the broadcast is and gives the tokens, and the chat is on MoQ tracks through the gatherer (`crates/chat`), on the session of the broadcast. **While the room has no broadcast**, the reader takes part in the chat on a session at the relay the room's hello names (`relay`, decided on 2026-10-25), with the chat's `sub` token in its SETUP; when a broadcast comes, it ends that session and connects to the broadcast. `xmoq-watch` and the pages wait for a broadcast as before.

Building it needs a C++ compiler, for the H.264 decoder that `xmoq-picture` builds (`openh264`), as `xmoq-watch` does.

## Using it from an agent

Run it in the background, and read the file:

```sh
xmoq-agent --room http://127.0.0.1:8787/rooms/demo --name claude --out /tmp/room &
tail -f /tmp/room/events.jsonl        # or wait for a line with a monitor
```

To say something, write a line to its stdin (a named pipe works: `mkfifo /tmp/room/say; xmoq-agent ... < /tmp/room/say &`, then `echo hello > /tmp/room/say`). Without stdin (`&` in a shell that gives it none) it reads on, and says nothing.

## Flags

```
xmoq-agent --room http://host:port/rooms/<name> --out <dir> [--name agent] [--token no]
xmoq-agent --url https://host:port/ --cert-sha256 <64 hex digits> [--namespace xmoq/demo]
           --chat-room <room> --out <dir> [--name agent]
```

| Flag | Default | What |
|---|---|---|
| `--room` | | The room. Its last path segment is the room of the chat |
| `--url`, `--cert-sha256`, `--namespace`, `--chat-room` | `xmoq/demo` for the namespace | A relay without a room, and the room of the chat on it |
| `--name` | `agent` | The name in the chat |
| `--out` | | The directory of `events.jsonl`, made where it is not. The file is added to, not begun anew |
| `--token no` | asked | Asks the room for no tokens |
| `--chat-catch-up-lines` | 16 | The lines missed while away that are fetched when the subscription to `lines` is made again; 1 to 1,000 (`history_to_fetch` of `xmoq-viewer`) |
| `--answer-wait-ms`, `--setup-wait-ms`, `--open-wait-ms` | 10,000 each | As `xmoq-watch` has them |
| `--seconds` | | Ends after so long |
| `--frames off` | on | The video is not subscribed to, and `/frame` gets an `error`. On, the reader subscribes to the first H.264 video track of the broadcast's catalog and holds the latest Group's frames undecoded until a picture is asked for (`xmoq-picture`) |
| `--frame-every-ms` | none | A picture this often by the broadcast's Timestamps, unasked |
| `--frame-longest` | 1280 | The longest side of a picture, in pixels; a larger one is made smaller |
| `--frames-kept` | 500 | The pictures kept in `<dir>/frames/`, of every session together; the oldest file goes for the newest. Files that were there when the reader began are not counted, and not removed |
| `--speech off` | on | No transcript is followed. On, the reader subscribes to `speech` of `xmoq/transcript/<prefix>`, held by the relay for 30 s for a transcriber to come, and asked for again when it is refused or ends |
| `--transcript` | the broadcast's namespace | The prefix the transcriber listens under (its `--namespace`), fields between slashes. With a room that gives tokens, the reader asks it for `sub` in `xmoq/transcript/<prefix>` |

## What it writes

Every line has `type` and `at`, the reader's clock in milliseconds since the Unix epoch when it wrote the line. The keys are in the order of their names. The types are `xmoq_agent::events::Event`, whose test has a line of each.

| `type` | Fields | When |
|---|---|---|
| `broadcast` | `state` (`began`, `ended`), `namespace` | The reader connects to the room's broadcast, and its session ends. With `--url`, the namespace of `--namespace`. Not for a session at the room's relay for the chat alone |
| `chat` | `number`, `from`, `text`, `sent_at` (the sender's clock), `gathered_at` (the gatherer's clock) | A line of the room, each once, in the order of its number: the latest 16 when the reader comes, then each as it is said. The reader's own lines come back as well |
| `chat_gap` | `after`, `before` | More lines were said while the reader was away than `--chat-catch-up-lines`: those between were not fetched |
| `joined`, `left` | `name` | Somebody's chat namespace `xmoq/chat/<room>/<name>` was announced, or withdrawn |
| `frame` | `path`, `group`, `object`, `timestamp_us` (the Timestamp of LOC of the frame), `width`, `height`, `asked` (`command`, `every`) | A picture of the broadcast, written as a PNG in `<dir>/frames/`: asked for with `/frame`, or every `--frame-every-ms`. It is of the latest frame that decoded, which can be older than the moment it was asked for. `/frame` that gets none within 10 s gets an `error` |
| `speech` | `number` (in the transcript), `from` (the namespace of the broadcast whose voice it was), `text`, `start_us`, `end_us` (on the clock of the Timestamps of LOC of that broadcast's audio), `read_at` (the transcriber's clock, ms) | Something said in a broadcast, as the transcriber (`xmoq-transcriber`) read it: the pieces of `speech` of `xmoq/transcript/<prefix>` (`xmoq-transcript-tracks`), each once, in the order of its number. The latest 16 when the reader comes, by a Joining FETCH, then each as it is read; those missed while away, as for the chat. Pieces not fetched are told as an `error`. With `--transcript`, the pieces of every broadcast under that prefix, each with its `from` |
| `error` | `message` | Something went wrong, or a line of stdin was not said; the reader goes on |

```json
{"at":1792883451092,"name":"p","type":"joined"}
{"at":1792883451113,"from":"p","gathered_at":1792883451111,"number":0,"sent_at":1792883451110,"text":"hello from p","type":"chat"}
{"asked":"command","at":1792884905154,"group":1792884901959,"height":360,"object":12,"path":"/tmp/room/frames/000001-g1792884901959-o12.png","timestamp_us":1792884904928000,"type":"frame","width":640}
{"at":1792883451182,"message":"no command /nothing: /frame is one, and // begins a line that is said","type":"error"}
{"at":1792883451205,"name":"p","type":"left"}
```

The text is as the sender wrote it: a line of JSON escapes what a terminal would act on, and nothing is taken out.

What the reader says about itself (the chat on tracks, a subscription asked for again) goes to stderr, not to the file.

## What a line of stdin does

| The line | What happens |
|---|---|
| `hello` | Said in the chat. Over 512 bytes, or while 16 lines wait for the gatherer, it is not, and an `error` says so |
| `/frame` | A picture of the broadcast now, as a `frame` line |
| `//hello` | `/hello` is said: one `/` is taken off, so that a line can begin with `/` |
| `/other` | An `error`: no such command |
| An empty line | Nothing |
| Over 4,096 bytes | Not read, and an `error` says so |

## Who is in the chat

`joined` and `left` come of a `SUBSCRIBE_NAMESPACE` for `xmoq/chat/<room>` (decided on 2026-10-25), as the gatherer finds its participants. **Only those who announce a namespace for the chat are seen**: a page or a viewer that does not take part in the chat is not. The reader itself is left out. Where the reader connects again, the relay tells of everybody there; those it does not tell of within 2 s (`SETTLE_MS`, **provisional**) are written as `left`.

## Known gaps

- **With a room that names no relay, the chat is there only while the room has a broadcast**: the room says where the relay is only with the broadcast then (a room run without `RELAY_URL`, `spikes/durable-objects-room/README.md`). A line said meanwhile gets an `error`.
- **The transcript is that of the broadcast's namespace unless `--transcript` names the prefix the transcriber listens under**: a transcriber run with a prefix above the broadcast's (`--namespace xmoq` for `xmoq/demo`) publishes `xmoq/transcript/xmoq`, and the reader without `--transcript xmoq` waits for `xmoq/transcript/xmoq/demo`, which does not come. The room does not say what the transcriber listens under. A session at the room's relay for the chat alone follows no transcript.
- **A reader started again in the same `--out` writes over the pictures of the one before**: the names begin from 1 again (`000001-...`), and what the earlier run kept is neither counted nor removed.
- The video that ends (PUBLISH_DONE) is not asked for again on that session; the next session asks.
- Objects of the video that wait for the thread of the pictures: 256; beyond them they are left out, an `error` says so when it begins, and the frames after them in their Group give no picture.
- Requests of the relay are refused with `NOT_SUPPORTED`, except the gatherer's SUBSCRIBE to `said`.

## What is where

The library (`src/*.rs`) decides, and does no input or output: `events` (the lines written), `input` (what a line of stdin asks for), `presence` (who is in the chat), `args` (the flags). The binary (`src/bin/xmoq-agent/`): `main.rs` (the room, connecting again, the signals), `attend.rs` (one session: the chat, who is in it, stdin, the video), `frames.rs` (the pictures, on a thread that writes the PNG files), `output.rs` (stdout and the file), `stdin.rs` (the lines of stdin, on a thread).

`interop/agent.ts` runs it against a page through the own relay and the gatherer (`mise run interop`). `interop/agent-frames.ts` asks it twice with `/frame` for a picture of a page's broadcast, and has the page decode the PNG ([record](../../docs/experiments/2026-10-25-pictures-of-a-broadcast-for-an-agent.md)).
