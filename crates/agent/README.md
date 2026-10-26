# xmoq-agent

A reader of a room for an agent. It joins a room as a participant of its chat, and writes what happens there as lines of JSON, one event a line, on stdout and at the end of `<dir>/events.jsonl`. A line of its stdin is said in the chat under its name.

It connects the way `xmoq-watch` does (`xmoq-viewer` and `xmoq-viewer-io` are the parts the two share): the room, asked over its HTTP API (`fixtures/room-api/README.md`), says where the relay and the broadcast are and gives the tokens, and the chat is on MoQ tracks through the gatherer (`crates/chat`), on the session of the broadcast. **The room names its broadcast whether it has begun or not**, and does not say when it begins: the SUBSCRIBE of the broadcast's catalog carries a `RENDEZVOUS_TIMEOUT` of 30 s (`RENDEZVOUS_MS` of `xmoq-viewer-io`, the value for a followed track decided on 2026-10-26, **provisional**), so that the relay holds it until the broadcast's publisher comes. **The session does not wait for it**: the catalog is asked for on the session's loop beside the chat and the transcript (`FollowedCatalog` of `xmoq-viewer-io`), kept as every other track is (`xmoq_viewer_io::kept`): one refused at the end of its rendezvous, or with `DOES_NOT_EXIST`, is asked again after the pause of `xmoq-again`, and one whose subscription ends (the broadcast left) is asked again as well. When the catalog comes, the reader writes `broadcast` `began` and subscribes to the video, on the same session. Without a room (`--url`) the catalog is asked for so too, with no rendezvous.

Building it needs a C++ compiler, for the H.264 decoder that `xmoq-picture` builds (`openh264`), as `xmoq-watch` does.

## Using it from an agent

`mise run agent-demo -- start --out /tmp/room` does all of what follows in one go, with a voice as the page's microphone, and `mise run agent-demo -- stop --out /tmp/room` ends it (`scripts/agent-demo.sh` has its flags). `start --meeting 3` opens 3 participants of the meeting (`/meet`) in place of the publishing page, each saying the voice 2 s after the one before, and starts the reader in the meeting's room (`/meetings/demo`); it ends by printing their ids, and `/frame <id>` gives the picture of one ([record](../../docs/experiments/2026-10-25-agent-demo-with-a-meeting.md)). By hand: a demo of its own, on ports apart from `mise run demo`'s (4443, 8080, 8787), then the reader in its room, in the background:

```sh
bun install --frozen-lockfile          # once after main moved: the demo bundles the packages
bun demo/serve.ts --relay-port 33443 --port 33080 --rooms-port 33787 --chat-store /tmp/room/chat.sqlite3 &
# add --transcribe-language en (or ja, ...) where the voice's language is known: twice as fast as auto
# open http://localhost:33080/publish?audio=mic in Chromium: the broadcast, with the microphone as its sound
mkdir -p /tmp/room; mkfifo /tmp/room/say; sleep 86400 > /tmp/room/say &
mise run agent -- --room http://127.0.0.1:33787/broadcasts/demo --name claude --out /tmp/room < /tmp/room/say &
echo 'hello' > /tmp/room/say          # said in the chat
echo '/frame' > /tmp/room/say         # a picture of the broadcast now
tail -f /tmp/room/events.jsonl
```

The lines of `events.jsonl` are `chat`, `joined`, `left`, `broadcast`, `frame`, `speech`, `chat_gap` and `error` (the table below). A `frame` line's `path` is the PNG's from `--out` (`frames/<when the reader began, ms>/<file>`, in `/tmp/room` here), as the page of a meeting writes it too (`fixtures/agent-lines/`). `speech` lines come where the demo has its transcriber (built with `cargo build --release -p xmoq-transcriber` and its models fetched with `mise run speech-model` and `mise run speech-model -- silero-v5.1.2`; the demo says so when it starts) and the page sends a voice: without `?audio=mic` it sends a tone, in which there are no words. The `sleep` holds the pipe open: without stdin the reader reads on and says nothing.

## Flags

The flags and what each is without it are `USAGE` in `src/args.rs`, which the reader prints when a flag is wrong. What they do together:

**A room or a relay.** `--room` is the room of a broadcast (`http(s)://host:port/broadcasts/<name>`) or of a meeting (`.../meetings/<name>`); the kind and the name are the chat's room (`broadcast/<name>`, `meeting/<name>`), and `--chat-room` is not taken with it. The room is asked for its relay before each session, with the broadcast's namespace or the meeting's, and the prefix of its transcript; with a room of a broadcast `--token no` asks it for no tokens. A room that refuses the connection is asked again every 1 s for 30 s, as `xmoq-watch` does (`ROOM_COMES_WITHIN` of `xmoq-viewer-io`). A room that fails before a session (it cannot be reached, does not answer within `--room-wait-ms`, answers what the reader does not take, names no relay) is asked again 1 s after the try before, with an `error` where the failure is not the one told last. A first session that fails does not end the reader (where `xmoq-watch` ends): an agent starts it, often as the relay and the room start. Without a room, `--url` and `--cert-sha256` name a relay, `--namespace` the broadcast (needed: there is no default, the caller names it; `xmoq/broadcast/demo` for the demo's), or `--meeting` a meeting, and `--chat-room broadcast/<name>` or `meeting/<name>` the room of the chat on it. The waits (`--answer-wait-ms`, `--setup-wait-ms`, `--open-wait-ms`, `--room-wait-ms`, `--connect-wait-ms`) are as `xmoq-watch` has them; when the next session is made, also after a GOAWAY, is `NextSession` of `xmoq-again`, as for `xmoq-watch` (its README, "The next session"). A SUBSCRIBE refused with `GOING_AWAY` waits for the GOAWAY behind it and moves.

**`--out`.** Started again in the same `--out`, the reader adds to `events.jsonl` and puts its pictures in a directory of its own (`frames/<when it began, ms>/`): nothing of a run before is written over or removed. `--frames-kept` counts the pictures of this run, of every session together, and the oldest file goes for the newest; the directories of runs before are not counted.

**Pictures.** When the broadcast's catalog comes, the reader subscribes to its first H.264 video track and holds the latest Group's frames undecoded until a picture is asked for (`xmoq-picture`); with `--frames off` it does not, and `/frame` gets an `error`. The video is kept on the session as `xmoq-watch` keeps its sound (`xmoq_viewer_io::kept`): a subscription that ends, is refused or gets a GOAWAY on its stream is asked for again, a SUBSCRIBE that could not be sent or got no answer gives the video up on that session, and a message on its stream that breaks the protocol closes the session. A picture larger than `--frame-longest` is made smaller.

**The chat.** `--chat-catch-up-lines` is how many of the lines missed while away are fetched when the subscription to `lines` is made again (`history_to_fetch` of `xmoq-viewer`).

**The transcript.** The reader subscribes to `speech` of `xmoq/transcript/<prefix>`, held by the relay for 30 s for a transcriber to come, and kept on the session as the video is, but for a SUBSCRIBE that could not be sent or got no answer, which is asked again as a refusal that names no time, as `@xmoq/transcript` does; `--speech off` follows none. The prefix is `--transcript` where it is given, else the `transcript` the room answers with (of the room of a broadcast, or of a meeting), else in a meeting `xmoq/meet/<room>`, else the broadcast's namespace (`xmoq_agent::transcript::prefix`, with why). With a room that gives tokens, the reader asks it for `sub` in `xmoq/transcript/<prefix>`.

**A meeting** (the room of a meeting, or `--meeting <room>` with `--url`) in place of one broadcast: the participants publish under `xmoq/meet/<room>/<id>`, and the reader is told of them with one `SUBSCRIBE_NAMESPACE` for `xmoq/meet/<room>`. It follows no participant's sound: the transcript is the one above, and a transcriber that listens under that prefix says whose voice each piece was. Without a room, the chat is `meeting/<room>` unless `--chat-room` names another. `/frame <id>` subscribes to that participant's video and gives its picture; `/frame` alone is of the participant whose voice was read last. **One participant's video at a time**: a picture of another cancels the subscription of the one before, and `--frames-kept` holds for all of them together. A participant's video stays subscribed for `--meeting-video-linger-ms` after the last picture of it was asked for, so that the next picture of the same participant needs no new subscription and no key frame to wait for. One participant more than `--meeting-most` is told of as an `error` and left out. **The seat.** In the room of a meeting (`--token no` is not taken there, nor `--meeting`), the reader takes a seat under a random id of its own (`POST .../seats`) and holds it while it runs: the seat is renewed on a task of its own as the room asks (`renewMs`), each session's SETUP and requests carry the seat's newest `sub` in `xmoq/meet/<room>/*`, and the reader leaves the seat when it ends, on a signal as well. What becomes of each answer that is no seat: a full meeting (`full`) ends the reader with an `error`; `taken` (another holds the id, or a seat of the reader's that it lost has not run out yet) takes a new id and asks again; `not-a-participant` and `not-a-meeting-room` end the reader (the flags are wrong). A seat that is lost (`gone`, or a renewal that fails) ends the session with an `error`, and the next session takes a seat again. The reader announces nothing under the meeting, and is not listed as a participant.

## What it writes

Every line has `type` and `at`, the reader's clock in milliseconds since the Unix epoch when it wrote the line. The keys are in the order of their names. The types are `xmoq_agent::events::Event`, whose test has a line of each.

| `type` | Fields | When |
|---|---|---|
| `broadcast` | `state` (`began`, `ended`), `namespace` | `began`: the broadcast's catalog came on the session, so the broadcast is there; `ended`: after `began`, the catalog's subscription ended (the broadcast left) or the session ended. Each `began` has its `ended`; a session in which the broadcast never came writes neither. With `--url`, the namespace of `--namespace`. Not in a meeting |
| `chat` | `number`, `from`, `text`, `sent_at` (the sender's clock), `gathered_at` (the gatherer's clock) | A line of the room, each once, in the order of its number: the latest 16 when the reader comes, then each as it is said. The reader's own lines come back as well |
| `chat_gap` | `after`, `before` | More lines were said while the reader was away than `--chat-catch-up-lines`: those between were not fetched |
| `joined`, `left` | `name` | Somebody's chat namespace `xmoq/chat/<kind>/<room>/<name>` was announced, or withdrawn |
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
| `/frame` | A picture of the broadcast now, as a `frame` line. Asked while the reader connects (again), it is kept and asked of the next session's video; within 10 s from when it was typed, or an `error`. Before the broadcast's catalog came, an `error` (`frame: no broadcast yet`) |
| `//hello` | `/hello` is said: one `/` is taken off, so that a line can begin with `/` |
| `/other` | An `error`: no such command |
| An empty line | Nothing |
| Over 4,096 bytes | Not read, and an `error` says so |

## Who is in the chat

`joined` and `left` come of a `SUBSCRIBE_NAMESPACE` for `xmoq/chat/<kind>/<room>` (decided on 2026-10-25), as the gatherer finds its participants. **Only those who announce a namespace for the chat are seen**: a page or a viewer that does not take part in the chat is not. The reader itself is left out. Where the reader connects again, the relay tells of everybody there; those it does not tell of within 2 s (`SETTLE_MS`, **provisional**) are written as `left`. The rule is `xmoq_viewer::presence`'s, which a meeting's participants go by as well. 4,096 are kept (`MOST_IN_THE_CHAT`, the namespaces the own relay tells of to one request; **provisional**); one more is written as an `error` and not as `joined`.

## Known gaps

- **A room that names no relay is asked again, and the reader has no chat meanwhile.**
- **Without `--transcript` and a room that names its transcript, the transcript is that of the broadcast's namespace**: a transcriber run with a prefix above the broadcast's (`--namespace xmoq` for `xmoq/broadcast/demo`) publishes `xmoq/transcript/xmoq`, and such a reader waits for `xmoq/transcript/xmoq/broadcast/demo`, which does not come.
- **In a room's meeting, a session that lasts longer than the seat's token is refused what it asks after**: the seat's renewals bring new tokens, and a session keeps the one it began with. A participant who comes after the token ran out, or a picture of one whose video was not subscribed to, is refused by a relay with `--auth-key-hex`. Not run.
- Objects of the video that wait for the thread of the pictures: 256; beyond them they are left out, an `error` says so when it begins, and the frames after them in their Group give no picture.
- Requests of the relay are refused with `NOT_SUPPORTED`, except the gatherer's SUBSCRIBE to `said`.

## What is where

The library (`src/*.rs`) decides, and does no input or output: `events` (the lines written), `input` (what a line of stdin asks for), `presence` (who is in the chat), `args` (the flags). The binary (`src/bin/xmoq-agent/`): `main.rs` (the room and its seat, connecting again, the signals), `attend.rs` (one session: the chat, who is in it, stdin, the video), `video.rs` (the catalog and the video's track, kept on the session), `frames.rs` (the pictures, on a thread that writes the PNG files), `output.rs` (stdout and the file), `stdin.rs` (the lines of stdin, on a thread).

`interop/agent.ts` runs it against a page through the own relay and the gatherer (`mise run interop`). `interop/agent-frames.ts` asks it twice with `/frame` for a picture of a page's broadcast, and has the page decode the PNG ([record](../../docs/experiments/2026-10-25-pictures-of-a-broadcast-for-an-agent.md)). `interop/agent-meeting.ts` runs it with `--meeting` and `--url` against two pages that publish under `xmoq/meet/r/<id>` ([record](../../docs/experiments/2026-10-25-an-agent-in-a-meeting.md)).
