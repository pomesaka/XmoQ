# xmoq-agent

A reader of a room for an agent. It joins a room as a participant of its chat, and writes what happens there as lines of JSON, one event a line, on stdout and at the end of `<dir>/events.jsonl`. A line of its stdin is said in the chat under its name.

It connects the way `xmoq-watch` does (`xmoq-viewer` and `xmoq-viewer-io` are the parts the two share): the room says where the broadcast is and gives the tokens, and the chat is on MoQ tracks through the gatherer (`crates/chat`), on the session of the broadcast. **While the room has no broadcast**, the reader takes part in the chat on a session at the relay the room's hello names (`relay`, decided on 2026-10-25), with the chat's `sub` token in its SETUP; when a broadcast comes, it ends that session and connects to the broadcast. `xmoq-watch` and the pages wait for a broadcast as before.

Building it needs a C++ compiler, for the H.264 decoder that `xmoq-picture` builds (`openh264`), as `xmoq-watch` does.

## Using it from an agent

`mise run agent-demo -- start --out /tmp/room` does all of what follows in one go, with a voice as the page's microphone, and `mise run agent-demo -- stop --out /tmp/room` ends it (`scripts/agent-demo.sh` has its flags). `start --meeting 3` opens 3 participants of the meeting (`/meet`) in place of the publishing page, each saying the voice 2 s after the one before, and starts the reader with `--meeting demo`; it ends by printing their ids, and `/frame <id>` gives the picture of one ([record](../../docs/experiments/2026-10-25-agent-demo-with-a-meeting.md)). By hand: a demo of its own, on ports apart from `mise run demo`'s (4443, 8080, 8787), then the reader in its room, in the background:

```sh
bun install --frozen-lockfile          # once after main moved: the demo bundles the packages
bun demo/serve.ts --relay-port 33443 --port 33080 --rooms-port 33787 --chat-store /tmp/room/chat.sqlite3 &
# add --transcribe-language en (or ja, ...) where the voice's language is known: twice as fast as auto
# open http://localhost:33080/publish?audio=mic in Chromium: the broadcast, with the microphone as its sound
mkdir -p /tmp/room; mkfifo /tmp/room/say; sleep 86400 > /tmp/room/say &
mise run agent -- --room http://127.0.0.1:33787/rooms/demo --name claude --out /tmp/room < /tmp/room/say &
echo 'hello' > /tmp/room/say          # said in the chat
echo '/frame' > /tmp/room/say         # a picture of the broadcast now
tail -f /tmp/room/events.jsonl
```

The lines of `events.jsonl` are `chat`, `joined`, `left`, `broadcast`, `frame`, `speech`, `chat_gap` and `error` (the table below). A `frame` line's `path` is the PNG's from `--out` (`frames/<when the reader began, ms>/<file>`, in `/tmp/room` here), as the page of a meeting writes it too (`fixtures/agent-lines/`). `speech` lines come where the demo has its transcriber (built with `cargo build --release -p xmoq-transcriber` and its models fetched with `mise run speech-model` and `mise run speech-model -- silero-v5.1.2`; the demo says so when it starts) and the page sends a voice: without `?audio=mic` it sends a tone, in which there are no words. The `sleep` holds the pipe open: without stdin the reader reads on and says nothing.

## Flags

```
xmoq-agent --room http://host:port/rooms/<name> --out <dir> [--name agent] [--token no | --meeting <name>]
xmoq-agent --url https://host:port/ --cert-sha256 <64 hex digits> [--namespace xmoq/demo]
           (--chat-room <room> | --meeting <room>) --out <dir> [--name agent]
```

| Flag | Default | What |
|---|---|---|
| `--room` | | The room. Its last path segment is the room of the chat. A room that refuses the connection is asked again every 1 s for 30 s, as `xmoq-watch` does (`ROOM_COMES_WITHIN` of `xmoq-viewer-io`) |
| `--url`, `--cert-sha256`, `--namespace`, `--chat-room` | `xmoq/demo` for the namespace | A relay without a room, and the room of the chat on it |
| `--name` | `agent` | The name in the chat |
| `--out` | | The directory of `events.jsonl` and `frames/`, made where it is not. Started again in the same `--out`, the reader adds to `events.jsonl` and puts its pictures in a directory of its own (`frames/<when it began, ms>/`): nothing of a run before is written over or removed |
| `--token no` | asked | Asks the room for no tokens |
| `--chat-catch-up-lines` | 16 | The lines missed while away that are fetched when the subscription to `lines` is made again; 1 to 1,000 (`history_to_fetch` of `xmoq-viewer`) |
| `--answer-wait-ms`, `--setup-wait-ms`, `--open-wait-ms` | 10,000 each | As `xmoq-watch` has them |
| `--room-wait-ms`, `--connect-wait-ms` | 5,000; 10,000 | How long the room gets for each of its answers (a token, a seat), and the relay to be connected to; as `xmoq-watch` has them (`xmoq_viewer::reconnect`). A room that went away during a session is joined again before the next one, and one that does not come asked again, with an `error` where the failure is not the one told last. When the next session is made, also after a GOAWAY, is `xmoq_viewer::reconnect`, as for `xmoq-watch` (its README, "The next session"). With a room, a first session that fails does not end the reader (where `xmoq-watch` ends): an agent starts it, often as the relay and the room start. A SUBSCRIBE refused with `GOING_AWAY` waits for the GOAWAY behind it and moves |
| `--seconds` | | Ends after so long |
| `--frames off` | on | The video is not subscribed to, and `/frame` gets an `error`. On, the reader subscribes to the first H.264 video track of the broadcast's catalog and holds the latest Group's frames undecoded until a picture is asked for (`xmoq-picture`). The video is kept on the session as `xmoq-watch` keeps its sound (`xmoq_viewer_io::kept`): a subscription that ends, is refused or gets a GOAWAY on its stream is asked for again, a SUBSCRIBE that could not be sent or got no answer gives the video up on that session, and a message on its stream that breaks the protocol closes the session |
| `--frame-every-ms` | none | A picture this often by the broadcast's Timestamps, unasked |
| `--frame-longest` | 1280 | The longest side of a picture, in pixels; a larger one is made smaller |
| `--frames-kept` | 500 | The pictures kept of this run, of every session together; the oldest file goes for the newest. The directories of runs before are not counted, and not removed |
| `--speech off` | on | No transcript is followed. On, the reader subscribes to `speech` of `xmoq/transcript/<prefix>`, held by the relay for 30 s for a transcriber to come, and kept on the session as the video is, but for a SUBSCRIBE that could not be sent or got no answer, which is asked again as a refusal that names no time, as `@xmoq/transcript` does |
| `--meeting` | | A meeting in place of one broadcast (**proposed**): the participants publish under `xmoq/meet/<room>/<id>`, and the reader is told of them with one `SUBSCRIBE_NAMESPACE` for `xmoq/meet/<room>`. It follows no participant's sound: the transcript is the one the room names for its meeting, else that of `xmoq/meet/<room>`, unless `--transcript` names another, and a transcriber that listens under that prefix says whose voice each piece was. The chat is the meeting's room unless `--chat-room` names another. `/frame <id>` subscribes to that participant's video and gives its picture; `/frame` alone is of the participant whose voice was read last. **One participant's video at a time**: a picture of another cancels the subscription of the one before, and `--frames-kept` holds for all of them together. With `--room`, the meeting is the room's (`--meeting` is its name, and `--token no` is not taken): the reader asks the room for a seat with `join-meeting` under a random id of its own, and is in the meeting at the relay the room's hello names, with the seat's `sub` in `xmoq/meet/<room>/*` in its SETUP and on its requests; a full meeting (`meeting-full`) ends the reader with an `error`. The seat is one of the room's 49 (`spikes/durable-objects-room/README.md`), asked for again with each session for a new token. The room's broadcast, if it has one, is not followed. The reader announces nothing under the meeting, and is not listed as a participant |
| `--meeting-most` | 49 | The participants kept, 1 or more; one more is told of as an `error` and left out |
| `--meeting-video-linger-ms` | 30,000 | How long a participant's video stays subscribed after the last picture of it was asked for. The next picture of the same participant needs no new subscription and no key frame to wait for |
| `--transcript` | the room's, else the meeting's, else the broadcast's namespace | The prefix the transcriber listens under (its `--namespace`), fields between slashes, where it is another than the room says. Without it, the prefix the room's hello names: with `--meeting` its `meetingTranscript` (the room's `MEETING_TRANSCRIPT_PREFIX`), else its `transcript` (`TRANSCRIPT_PREFIX`); else with `--meeting` `xmoq/meet/<room>`, else the broadcast's namespace (`xmoq_agent::transcript::prefix` has the order). With a room that gives tokens, the reader asks it for `sub` in `xmoq/transcript/<prefix>` |

## What it writes

Every line has `type` and `at`, the reader's clock in milliseconds since the Unix epoch when it wrote the line. The keys are in the order of their names. The types are `xmoq_agent::events::Event`, whose test has a line of each.

| `type` | Fields | When |
|---|---|---|
| `broadcast` | `state` (`began`, `ended`), `namespace` | The reader connects to the room's broadcast, and its session ends. With `--url`, the namespace of `--namespace`. Not for a session at the room's relay for the chat alone |
| `chat` | `number`, `from`, `text`, `sent_at` (the sender's clock), `gathered_at` (the gatherer's clock) | A line of the room, each once, in the order of its number: the latest 16 when the reader comes, then each as it is said. The reader's own lines come back as well |
| `chat_gap` | `after`, `before` | More lines were said while the reader was away than `--chat-catch-up-lines`: those between were not fetched |
| `joined`, `left` | `name` | Somebody's chat namespace `xmoq/chat/<room>/<name>` was announced, or withdrawn |
| `participant` | `state` (`joined`, `left`), `id` | With `--meeting`: a participant's broadcast `xmoq/meet/<room>/<id>` was announced, or withdrawn; on a new session, those not told of again within 2 s are written as left. `id` is not the participant's name in the chat |
| `frame` | `path` (from `--out`, with `/`: `frames/<when the run began, ms>/<file>`), `group`, `object`, `timestamp_us` (the Timestamp of LOC of the frame), `width`, `height`, `asked` (`command`, `every`); with `--meeting`, `id`, whose picture it is | A picture of the broadcast, written as a PNG in `<dir>/frames/<when the run began, ms>/`: asked for with `/frame`, or every `--frame-every-ms`. It is of the latest frame that decoded, which can be older than the moment it was asked for. `/frame` that gets none within 10 s gets an `error` |
| `speech` | `number` (in the transcript), `from` (the namespace of the broadcast whose voice it was), `text`, `start_us`, `end_us` (on the clock of the Timestamps of LOC of that broadcast's audio), `read_at` (the transcriber's clock, ms) | Something said in a broadcast, as the transcriber (`xmoq-transcriber`) read it: the pieces of `speech` of `xmoq/transcript/<prefix>` (`xmoq-transcript-tracks`), each once, in the order of its number. The latest 16 when the reader comes, by a Joining FETCH, then each as it is read; those missed while away, as for the chat. Pieces not fetched are told as an `error`. With `--transcript`, the pieces of every broadcast under that prefix, each with its `from` |
| `error` | `message` | Something went wrong, or a line of stdin was not said; the reader goes on |

```json
{"at":1792883451092,"name":"p","type":"joined"}
{"at":1792883451113,"from":"p","gathered_at":1792883451111,"number":0,"sent_at":1792883451110,"text":"hello from p","type":"chat"}
{"asked":"command","at":1792884905154,"group":1792884901959,"height":360,"object":12,"path":"frames/1792884900120/000001-g1792884901959-o12.png","timestamp_us":1792884904928000,"type":"frame","width":640}
{"at":1792883451182,"message":"no command /nothing: /frame is one, and // begins a line that is said","type":"error"}
{"at":1792883451205,"name":"p","type":"left"}
```

The text is as the sender wrote it: a line of JSON escapes what a terminal would act on, and nothing is taken out.

What the reader says about itself (the chat on tracks, a subscription asked for again) goes to stderr, not to the file.

## What a line of stdin does

| The line | What happens |
|---|---|
| `hello` | Said in the chat. Over 512 bytes, or while 16 lines wait for the gatherer, it is not, and an `error` says so |
| `/frame` | A picture of the broadcast now, as a `frame` line. Asked while the reader connects (again), it is kept and asked of the next session's video; within 10 s from when it was typed, or an `error` |
| `//hello` | `/hello` is said: one `/` is taken off, so that a line can begin with `/` |
| `/other` | An `error`: no such command |
| An empty line | Nothing |
| Over 4,096 bytes | Not read, and an `error` says so |

## Who is in the chat

`joined` and `left` come of a `SUBSCRIBE_NAMESPACE` for `xmoq/chat/<room>` (decided on 2026-10-25), as the gatherer finds its participants. **Only those who announce a namespace for the chat are seen**: a page or a viewer that does not take part in the chat is not. The reader itself is left out. Where the reader connects again, the relay tells of everybody there; those it does not tell of within 2 s (`SETTLE_MS`, **provisional**) are written as `left`. The rule is `xmoq_viewer::presence`'s, which a meeting's participants go by as well. 4,096 are kept (`MOST_IN_THE_CHAT`, the namespaces the own relay tells of to one request; **provisional**); one more is written as an `error` and not as `joined`.

## Known gaps

- **With a room that names no relay, the chat is there only while the room has a broadcast**: the room says where the relay is only with the broadcast then (a room run without `RELAY_URL`, `spikes/durable-objects-room/README.md`). A line said meanwhile gets an `error`.
- **Without `--transcript` and a room that names its transcript, the transcript is that of the broadcast's namespace**: a transcriber run with a prefix above the broadcast's (`--namespace xmoq` for `xmoq/demo`) publishes `xmoq/transcript/xmoq`, and such a reader waits for `xmoq/transcript/xmoq/demo`, which does not come. A session at the room's relay for the chat alone follows the transcript the flag or the room names, and none else.
- **In a room's meeting, a session that lasts longer than the seat's token is refused what it asks after**: the room's tokens are good for 600 s, and the seat is asked for again only when a session begins. A participant who comes after that, or a picture of one whose video was not subscribed to, is refused by a relay with `--auth-key-hex`. Not run.
- Objects of the video that wait for the thread of the pictures: 256; beyond them they are left out, an `error` says so when it begins, and the frames after them in their Group give no picture.
- Requests of the relay are refused with `NOT_SUPPORTED`, except the gatherer's SUBSCRIBE to `said`.

## What is where

The library (`src/*.rs`) decides, and does no input or output: `events` (the lines written), `input` (what a line of stdin asks for), `presence` (who is in the chat), `args` (the flags). The binary (`src/bin/xmoq-agent/`): `main.rs` (the room, connecting again, the signals), `attend.rs` (one session: the chat, who is in it, stdin, the video), `video.rs` (the catalog and the video's track, kept on the session), `frames.rs` (the pictures, on a thread that writes the PNG files), `output.rs` (stdout and the file), `stdin.rs` (the lines of stdin, on a thread).

`interop/agent.ts` runs it against a page through the own relay and the gatherer (`mise run interop`). `interop/agent-frames.ts` asks it twice with `/frame` for a picture of a page's broadcast, and has the page decode the PNG ([record](../../docs/experiments/2026-10-25-pictures-of-a-broadcast-for-an-agent.md)). `interop/agent-meeting.ts` runs it with `--meeting` against two pages that publish under `xmoq/meet/r/<id>` ([record](../../docs/experiments/2026-10-25-an-agent-in-a-meeting.md)).
