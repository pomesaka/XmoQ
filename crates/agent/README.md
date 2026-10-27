# xmoq-agent

A reader of a room for an agent. It joins a room as a participant of its chat, and writes what happens there as lines of JSON, one event a line, on stdout at once and in the files of `<dir>/events/`, a file a second. A line of its stdin, or one added to `<dir>/inbox.txt`, is said in the chat under its name.

It connects the way `xmoq-watch` does (`xmoq-viewer`, `xmoq-viewer-io` and `xmoq-subscriber` are the parts the two share): the room, asked over its HTTP API (`fixtures/room-api/README.md`), says where the relay and the broadcast are and gives the tokens, and the chat is on MoQ tracks through the gatherer (`crates/chat`), on the session of the broadcast. **The session does not wait for the broadcast**: its catalog is followed on the session's loop beside the chat and the transcript (`FollowedCatalog` of `xmoq-subscriber`), and when it comes the reader writes `broadcast` `began` and subscribes to the video, on the same session.

Building it needs a C++ compiler, for the H.264 decoder that `xmoq-picture` builds (`openh264`), as `xmoq-watch` does.

## Using it from an agent

How to run it beside a demo, with `mise run agent-demo` or by hand, is in [the demo's README](../../demo/README.md#an-agent-in-a-room). The files and lines it writes in `events/`, and what a line of its stdin or of `inbox.txt` does, are in [`fixtures/agent-lines/`](../../fixtures/agent-lines/README.md), the same as the page of a meeting writes and reads. **An agent that did not start the reader speaks through `inbox.txt`**: stdin is the starter's. Without stdin the reader reads on. What the reader says about itself (the chat on tracks, a subscription asked for again) goes to stderr, not to the files.

## Flags

The flags and what each is without it are `USAGE` in `src/args.rs`, which the reader prints when a flag is wrong. What they do together:

**A room or a relay.** `--room` is the room of a broadcast (`http(s)://host:port/broadcasts/<name>`) or of a meeting (`.../meetings/<name>`); the kind and the name are the chat's room (`broadcast/<name>`, `meeting/<name>`), and `--chat-room` is not taken with it. The room is asked for its relay before each session. **A first session that fails does not end the reader**, where `xmoq-watch` ends: an agent starts it, often as the relay and the room start (why, in `src/bin/xmoq-agent/main.rs`). Without a room, `--url` and `--cert-sha256` name a relay, `--namespace` the broadcast (needed: there is no default, the caller names it; `xmoq/broadcast/demo` for the demo's), or `--meeting` a meeting, and `--chat-room broadcast/<name>` or `meeting/<name>` the room of the chat on it.

**`--out`.** Started again in the same `--out`, the reader names its files of `events/` after the newest there and puts its pictures in a directory of its own (`frames/<when it began, ms>/`): nothing of a run before is written over or removed. What is in `inbox.txt` when it begins is passed over. `--frames-kept` counts the pictures of this run, of every session together, and the oldest file goes for the newest; the directories of runs before are not counted.

**Pictures.** The reader subscribes to the first H.264 video track of the broadcast's catalog and holds the latest Group's frames undecoded until a picture is asked for (`xmoq-picture`); with `--frames off` it does not, and `/frame` gets an `error`.

**The transcript.** The reader follows `speech` of `xmoq/transcript/<prefix>`; `--speech off` follows none. Which prefix, and why, is `xmoq_agent::transcript::prefix`.

**A meeting** (the room of a meeting, or `--meeting <room>` with `--url`) in place of one broadcast: the participants publish under `xmoq/meet/<room>/<id>`, and the reader is told of them with one `SUBSCRIBE_NAMESPACE` for `xmoq/meet/<room>`. It follows no participant's sound: the transcript is the one above, and a transcriber that listens under that prefix says whose voice each piece was. Without a room, the chat is `meeting/<room>` unless `--chat-room` names another. `/frame <id>` gives that participant's picture, and `/frame` alone that of the participant whose voice was read last. **One participant's video at a time** (why, in `xmoq_viewer::meeting`): `--frames-kept` holds for all of them together. In the room of a meeting the reader takes a seat under a random id of its own and holds it while it runs, and leaves it when it ends, on a signal as well; what becomes of each answer that is no seat is `after_room_failure` in `src/bin/xmoq-agent/main.rs`. **The reader announces nothing under the meeting, and is not listed as a participant.**

## Who is in the chat

`joined` and `left` come of a `SUBSCRIBE_NAMESPACE` for `xmoq/chat/<kind>/<room>`, as the gatherer finds its participants. **Only those who announce a namespace for the chat are seen**: a page or a viewer that does not take part in the chat is not. The reader itself is left out. The rule is `xmoq_viewer::presence`'s, which a meeting's participants go by as well. What each thing on the stream of the chat's or a meeting's `SUBSCRIBE_NAMESPACE` does, and when it is asked for again, is `KeptNamespace::on_stream` of `xmoq-subscriber` (`crates/subscriber/src/namespaces.rs`).

## Known gaps

- **A room that names no relay is asked again, and the reader has no chat meanwhile.**
- **Without `--transcript` and a room that names its transcript, the transcript is that of the broadcast's namespace**: a transcriber run with a prefix above the broadcast's (`--namespace xmoq` for `xmoq/broadcast/demo`) publishes `xmoq/transcript/xmoq`, and such a reader waits for `xmoq/transcript/xmoq/broadcast/demo`, which does not come.
- **In a room's meeting, a session that lasts longer than the seat's token is refused what it asks after**: the seat's renewals bring new tokens, and a session keeps the one it began with. A participant who comes after the token ran out, or a picture of one whose video was not subscribed to, is refused by a relay with `--auth-key-hex`. Not run.
- Objects of the video that wait for the thread of the pictures are bounded (`MOST_WAITING` in `src/bin/xmoq-agent/frames.rs`); beyond them they are left out, an `error` says so when it begins, and the frames after them in their Group give no picture.
- Requests of the relay are refused with `NOT_SUPPORTED`, except the gatherer's SUBSCRIBE to `said`.
