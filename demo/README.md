# Demo

A page publishes video through the own relay, and other pages watch it.

Two applications share the relay, the rooms, the gatherer of the chat and the transcriber that `serve.ts` starts: the broadcast (`broadcast/`: `/publish` and `/watch`) and the meeting (`meeting/`: `/meet`). Each puts its pages together in its own directory (`pages.ts`, the pages, the room it asks); `lib/` has what the two share within the demo, and what a third application would use is in `packages/` (the chat of a page, `showChat` of `@xmoq/chat`; the captions, `showCaptions` of `@xmoq/transcript`).

```sh
mise run demo
```

Then, in Chromium (the pages need WebTransport and WebCodecs with H.264):

1. Open <http://localhost:8080/publish>. It paints a clock and publishes it; <http://localhost:8080/publish?source=camera> publishes the camera.
2. Open <http://localhost:8080/watch> in one or more other tabs. A viewer that comes while another watches fetches the Group so far from the relay and has a picture at once; <http://localhost:8080/watch?join=no> waits for the next key frame instead, a second at most.

## What is worth a try now

With `mise run demo` up; each line is one thing to open, and what nobody has judged by eye and ear yet. The flags of each page are at the top of the file that reads them (`broadcast/watch.ts`, `broadcast/publish.ts`, `meeting/meet.ts`).

- <http://localhost:8080/publish?source=camera&audio=mic>: a real camera and microphone, which so far ran only as Chromium's fake devices. Left on for half an hour beside a watching page it also answers whether the two are timed by one clock ([the steps](../docs/experiments/2026-10-13-a-publisher-and-a-viewer-whose-clocks-run-at-different-speeds.md#for-a-person-with-a-camera-and-a-microphone-do-the-two-tracks-clocks-move-apart)).
- <http://localhost:8080/watch?buffer=50> and <http://localhost:8080/watch?buffer=auto> beside the plain <http://localhost:8080/watch>: what is missing is how the automatic one sounds while it moves ([record](../docs/experiments/2026-10-13-the-default-playout-buffer.md), with a link to listen through).
- <http://localhost:8080/publish?source=camera&size=720> (or `size=1080`): the camera at a larger picture ([record](../docs/experiments/2026-10-12-the-demo-at-720-and-1080-lines.md)).
- <http://localhost:8080/publish?ladder=1> with <http://localhost:8080/watch?abr=1&judge=try>: two versions of the video, and a viewer that changes between them (a trial; it shows on a narrow link; [record](../docs/experiments/2026-10-12-two-versions-of-the-video-and-a-viewer-that-changes-between-them.md), [and](../docs/experiments/2026-10-12-a-try-before-going-up-that-comes-again-as-soon.md)).
- `mise run watch -- --room http://127.0.0.1:8787/broadcasts/demo`, in Ghostty or Kitty: the same broadcast in the terminal, with the chat of the room under the picture.

**The publishing page may be a tab in the background** (why, at `every` in `lib/page.ts`). The first line under each picture has the frames a second, and the watching page a line on the sound: what goes to the loudspeaker, and how loud. The second line under the watching page's picture is **where the delay comes from**: how long a frame took to get there (encoder, link, relay, decoder), how long it waited in the buffer, and how long for the next repaint. On one machine nearly all of it is the buffer, which is set with the buttons and the slider beside "sound" while watching (why its default, at `DEFAULT_PLAYOUT_BUFFER_MS` in `packages/media/src/core/playout-clock.ts`). With a camera, the publishing page says how long a frame took from the camera to the encoder where the browser tells; that part is not in the viewer's numbers. Every 2 seconds the publishing page shows a white square and sounds a beep at the same moment; the lines under the picture say how far apart the two came out.

**On a bad link**, `bun demo/serve.ts --congestion bbr` chooses the relay's congestion control ([record](../docs/experiments/2026-10-11-the-demo-over-a-lossy-link-with-bbr.md)), and `--delivery-patience-ms 500` starts the relay with its experiment of giving up a Group a viewer does not have in time ([record](../docs/experiments/2026-10-12-a-page-behind-a-link-with-too-little-room-and-a-relay-that-gives-groups-up.md)); `/publish?picture=busy&bitrate=2500000` is a picture that fills its bitrate, and `--relay-log FILE` keeps the relay's lines. The watching page asks the relay to give late video up (why its timeout, at `VIDEO_TIMEOUT_MS` in `broadcast/watch.ts`; [what the relay does](../docs/experiments/2026-10-12-the-relay-follows-the-delivery-timeout-a-viewer-asks-for.md), [a timeout changed while it plays](../docs/experiments/2026-10-12-a-viewer-changes-its-delivery-timeout-with-request-update.md)). How a viewer that comes late gets its first picture: [record](../docs/experiments/2026-10-12-the-first-picture-of-a-viewer-who-comes-late-fetch-or-wait.md). A viewer that comes before the publisher waits for it at the relay ([record](../docs/experiments/2026-10-13-a-page-that-waits-for-the-broadcast-at-the-relay.md)).

**The chat goes over MoQ tracks**: `mise run demo` starts the gatherer of the chat (`xmoq-chat`) beside the relay, with its store in `demo/.chat/chat.sqlite3` (not committed; `--chat-store FILE` for another), and each page is a participant over a connection of its own (`keepInTheRoom` of [`@xmoq/chat`](../packages/chat/README.md)). Every line shows the time the gatherer took it and its number in the room. The limits of a line are flags of `serve.ts` (below), which it gives the gatherer and the pages alike. Where the gatherer is not built, the pages have no chat ([records](../docs/experiments/2026-10-24-demo-chat-over-tracks.md), [and](../docs/experiments/2026-10-24-the-chat-off-the-rooms-websocket.md)). Two pages with the same name (two publishing pages are both `publisher`) announce the same namespace and publish the same track `said`; what the relay and the gatherer make of the two was not tried. Without `--no-auth`, a gatherer built before it took `--auth-key-hex` is not started (`serve.ts` asks the binary), and the pages then have no chat, as the line under the picture says ([record](../docs/experiments/2026-10-24-the-demo-chat-with-tokens.md)).

**Captions from the transcriber**: `mise run demo` also starts the transcriber (`xmoq-transcriber`, a release build) beside the relay, where its models are in the cache (`mise run speech-model` and `mise run speech-model -- silero-v5.1.2`; without them the line under the picture says so, and the demo goes on without captions). It listens under `xmoq/broadcast/demo`, the broadcast of the room "demo", and the watching page shows each piece it reads under the picture (`showCaptions` of [`@xmoq/transcript`](../packages/transcript/)). **Open `/publish?audio=mic` and speak**: the tone the publishing page sends otherwise is told to be no voice, and no caption comes. `--transcribe-language ja` (or another code) reads faster than `auto`, and has a voice in another language come out in that one ([record](../docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md)).

**A meeting** (`/meet`): open <http://localhost:8080/meet> in two or more tabs. Each tab is a participant of the meeting of the room `?room=NAME` ("demo" when left out); what it publishes and shows is at the top of `meeting/meet.ts`. The sound of the others plays after a click on **sound**. The transcriber listens under `xmoq/meet/demo` as well, and the room names `xmoq/meet/<room>` as the transcript of its meeting (`MEETING_TRANSCRIPT_PREFIX`) beside `xmoq/broadcast/<room>` as that of its broadcast (`TRANSCRIPT_PREFIX`), so an agent in the meeting (`mise run agent -- --room http://127.0.0.1:8787/meetings/demo --out DIR`) gets what each one says, and one in the broadcast what the publisher says. The meetings of other rooms are not transcribed ([record](../docs/experiments/2026-10-25-the-demo-has-a-meeting-page.md)). The page has the room's chat under its id, as the other pages have it. **Claude Code without the CLI** (Chromium alone): **link a folder for Claude Code** has the person choose `~/Documents/xmoq` once (make it first; Chromium lets a page have no Documents folder itself), and **call Claude Code** writes the meeting into `~/Documents/xmoq/<room>/<id>/` as `xmoq-agent --out` writes (`events/`, a file a second, and `frames/`), with the meeting's transcript, and does what is added to `inbox.txt` there ([`fixtures/agent-lines/`](../fixtures/agent-lines/README.md)); pressed again it stops. The page says which folder; tell Claude Code to read the files of `events/` in the order of their names, and to read the new ones as they come. An experiment: what the others say is kept on the person's disk without their knowing, and Claude's lines are said under the person's name ([`/meeting`](../packages/meeting/README.md), [record](../docs/experiments/2026-10-25-a-page-writes-for-claude-code.md)).

`bun demo/check.ts` checks a running demo in headless Chromium: the frames a second, and that sound leaves the watching page.

Ctrl-C stops the web server, the relay, the gatherer and the transcriber. `bun demo/serve.ts --port N --relay-port M` chooses other ports.

On the watching page, click "sound" to hear the publishing page's tone: a browser lets a page make sound only after a click. The publishing page puts out an MSF catalog with its tracks, and the watching page plays the first video and the first audio track it names; others can read it: `xmoq-moqt-client --catalog-video`, and `mlmsub -videoname video -audioname audio` of moqlivemock ([record](../docs/experiments/2026-10-12-the-demo-has-a-catalog.md)).

**A room** comes with it: [`@xmoq/room`](../packages/room/README.md), run locally with Miniflare (its dependencies are the workspace's, which `mise run demo` installs), asked over HTTP. The publishing page publishes in the namespace its room names, `xmoq/broadcast/<room>`, and the watching page subscribes there and waits at the relay for a publisher, so the two may be opened in either order. **How many are in the room, and reactions**, go over tracks of the gatherer of the chat ([`xmoq-chat-tracks`](../crates/chat-tracks/README.md)): the line under the picture says how many are in the room's chat, the participants the gatherer follows (a page that is open, a terminal viewer and an agent that take part in the chat; a name twice is one), and an emoji somebody presses with the buttons under the chat stands on every page of the room for a while, those of one while of the gatherer as one with how many.

**A session that ends** (the relay stopped, the link gone for more than 6 seconds): both pages say so in their first line and connect again (`keepPublishing` and `keepPlaying` of `@xmoq/media`).

**Tokens are asked for** (who may get one is [undecided](../docs/open-questions.md)): `bun demo/serve.ts` makes a key for each run and gives it to the relay and to the rooms. The relay refuses a request without a token; the pages ask the room for one (`pub` to publish, `sub` to watch) and give it to their connection, which puts it into every request. The relay looks at the token in SETUP as well, and closes a session without one: `?token=no` on a page asks for none, and the page says that the relay closed the session as not authorized. `--no-auth` runs with no key, and the relay takes every request: for debugging and experiments.

## An agent in a room

`mise run agent-demo -- start --out /tmp/room` does all of what follows in one go, with a voice as the page's microphone, and `mise run agent-demo -- stop --out /tmp/room` ends it (`scripts/agent-demo.sh` has its flags). `start --meeting 3` opens 3 participants of the meeting (`/meet`) in place of the publishing page, each saying the voice 2 s after the one before, and starts the reader in the meeting's room (`/meetings/demo`); it ends by printing their ids, and `/frame <id>` gives the picture of one ([record](../docs/experiments/2026-10-25-agent-demo-with-a-meeting.md)). By hand: a demo of its own, on ports apart from `mise run demo`'s (4443, 8080, 8787), then the reader in its room, in the background:

```sh
bun install --frozen-lockfile          # once after main moved: the demo bundles the packages
bun demo/serve.ts --relay-port 33443 --port 33080 --rooms-port 33787 --chat-store /tmp/room/chat.sqlite3 &
# add --transcribe-language en (or ja, ...) where the voice's language is known: twice as fast as auto
# open http://localhost:33080/publish?audio=mic in Chromium: the broadcast, with the microphone as its sound
mise run agent -- --room http://127.0.0.1:33787/broadcasts/demo --name claude --out /tmp/room < /dev/null > /tmp/room/agent.log 2>&1 &
echo 'hello' >> /tmp/room/inbox.txt   # said in the chat, within a second
echo '/frame' >> /tmp/room/inbox.txt  # a picture of the broadcast now
cat /tmp/room/events/*.jsonl          # what happened, a file a second; agent.log has each line at once
```

`speech` lines come where the demo has its transcriber (built with `cargo build --release -p xmoq-transcriber` and its models fetched with `mise run speech-model` and `mise run speech-model -- silero-v5.1.2`; the demo says so when it starts) and the page sends a voice: without `?audio=mic` it sends a tone, in which there are no words. A line of the reader's stdin is said as well; `agent-demo` says through a pipe. What the reader writes is in [`fixtures/agent-lines/`](../fixtures/agent-lines/README.md).

## The terminal viewer beside a demo

`broadcast/scripts/` runs `xmoq-watch` (a release build) against a demo of its own, its publishing page in headless Chromium (`publish-headless.ts`): `try.sh` plays it for some seconds and prints what the viewer cost, `try-room.sh` joins it through the room of the broadcast with a token and once more without one, `try-keys.sh` types the keys on a pseudo-terminal (`keys-on-a-pty.py`), and `try-narrow-link.sh` puts a link with less room than the video between the relay and the viewer. `try-silent-peers.sh` starts no demo: it has the viewer wait for a room and a relay that answer nothing. `watch-demo.sh` plays `mise run demo` without a room. Each says how to run it at its top.

## Where it ends

- **One machine.** No run over a real network, between two machines, or through a NAT; the hosting is undecided.
- **A person watched twice**, and saw a viewer's picture, heard the sound and saw the terminal viewer's picture in Ghostty. The sync, the flags and the bad links nobody judged; every other statement about picture and sound is a count read from the page.
- **A frame that does not come is not concealed.** The end of a Group that comes after the next has begun is dropped; a piece of sound that is missing leaves a gap.
- **The camera and the microphone ran only as Chromium's fake devices.**
- **A publisher whose video and sound are timed by two clocks is handled only behind a flag** (`/publish?stamp=least`), and was not measured with real devices ([record](../docs/experiments/2026-10-13-a-publisher-and-a-viewer-whose-clocks-run-at-different-speeds.md)).

## Flags

The flags of a page are queries of its address, at the top of the file that reads them (`broadcast/watch.ts`, `broadcast/publish.ts`, `meeting/meet.ts`; those every page takes, `broadcast/room.ts` and `lib/`).

`bun demo/serve.ts` (`mise run demo`): its flags, each with what it does and what it is without it, are the options of `parseArgs` at the top of `demo/serve.ts`. **The values the applications choose are its flags too**, so that a run can choose others: the limits of a line of the chat and of reactions (`--chat-text-bytes`, `--chat-lines-per-second`, `--chat-reactions-per-second`), how long the gatherer counts the reactions of a room before it publishes them and how many emoji one while has (`--reaction-batch-ms`, `--reaction-kinds`), whose video a page of the meeting receives (`--meeting-most-watched`), and the transcriber's models (`--speech-model`, `--vad-model`). What the meeting's room says is not a flag: a page of the meeting keeps as many others as the room has seats, less itself.

