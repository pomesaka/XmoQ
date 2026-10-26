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

With `mise run demo` up; each line is one thing to open, and what nobody has judged by eye and ear yet. [Every flag](#every-flag) is below.

- <http://localhost:8080/publish?source=camera&audio=mic>: a real camera and microphone, which so far ran only as Chromium's fake devices. Left on for half an hour beside a watching page it also answers whether the two are timed by one clock ([the steps](../docs/experiments/2026-10-13-a-publisher-and-a-viewer-whose-clocks-run-at-different-speeds.md#for-a-person-with-a-camera-and-a-microphone-do-the-two-tracks-clocks-move-apart)).
- <http://localhost:8080/watch?buffer=50> and <http://localhost:8080/watch?buffer=auto> beside the plain <http://localhost:8080/watch> (150 ms, the default since 2026-10-21): what is missing is how the automatic one sounds while it moves ([record](../docs/experiments/2026-10-13-the-default-playout-buffer.md), with a link to listen through).
- <http://localhost:8080/publish?source=camera&size=720> (or `size=1080`): the camera at a larger picture.
- <http://localhost:8080/publish?ladder=1> with <http://localhost:8080/watch?abr=1&judge=try>: two versions of the video, and a viewer that changes between them (a trial; it shows on a narrow link).
- `mise run watch -- --room http://127.0.0.1:8787/broadcasts/demo`, in Ghostty or Kitty: the same broadcast in the terminal, with the chat of the room under the picture.

**The publishing page may be a tab in the background**: its clock runs in a worker, which a browser does not slow down as it does the timers of a page nobody looks at. (By the timers of the page, a publisher in a background tab sends one frame a second, and no sound in time.) The first line under each picture has the frames a second, and the watching page a line on the sound: what goes to the loudspeaker, and how loud.

**Where the delay comes from** is the second line under the watching page's picture: how long a frame took to get there (encoder, link, relay, decoder), how long it waited in the buffer, and how long for the next repaint. On one machine nearly all of it is the buffer. The buffer is set with the buttons and the slider beside "sound" while watching (or `?buffer=50`); "auto" (`?buffer=auto`, a trial, not the default) lets it follow how late things come, 400 ms at most. The default is 150 ms, decided on 2026-10-21 ([why](../packages/media/README.md#how-picture-and-sound-are-played)). The sound waits 30 ms at least, also where the buffer is less: with no wait at all its pieces come after their time and nothing sounds. With a camera, the publishing page says how long a frame took from the camera to the encoder where the browser tells; that part is not in the viewer's numbers.

**On a link that loses packets** the relay's congestion control decides whether the picture stays live: `bun demo/serve.ts --congestion bbr` (the relay's default is cubic; [record](../docs/experiments/2026-10-11-the-demo-over-a-lossy-link-with-bbr.md)). `/publish?picture=busy&bitrate=2500000` is a picture that fills its bitrate, and `--relay-log FILE` keeps the relay's lines.

**On a link with less room than the video** `bun demo/serve.ts --delivery-patience-ms 500` starts the relay with its experiment of giving up a Group a viewer does not have in time: the picture then stays about half a second behind and does not fall further, and the page shows more frames than without ([record](../docs/experiments/2026-10-12-a-page-behind-a-link-with-too-little-room-and-a-relay-that-gives-groups-up.md)).

`/publish?size=720` and `size=1080` send 1280x720 and 1920x1080 where the page otherwise sends 640x360, at 3 and 6 Mbit/s unless `?bitrate=` says otherwise, and ask a camera for that size ([record](../docs/experiments/2026-10-12-the-demo-at-720-and-1080-lines.md)).

**The watching page asks the relay to give late video up**: its SUBSCRIBE for the video carries a SUBGROUP_DELIVERY_TIMEOUT of 150 ms (transport §8), so a Group that is not through 150 ms after its end is reset for this viewer, and the picture stays near on a link that is narrower than the video, where it otherwise falls behind without end. Nothing is asked for the sound. The 150 ms were decided on 2026-10-21 for this page; `playVideoTrack` asks for nothing unless told, and whether it should is open. `/watch?videoTimeout=N` asks for another time and `?videoTimeout=0` for none; `?audioTimeout=N` asks for the sound; `?videoTimeoutThen=N&videoTimeoutAfter=S` changes the video's timeout to `N` ms after `S` seconds without subscribing anew, with a REQUEST_UPDATE ([record](../docs/experiments/2026-10-12-a-viewer-changes-its-delivery-timeout-with-request-update.md)) ([why 150](../docs/experiments/2026-10-12-which-delivery-timeout-a-viewer-asks-for-on-its-video.md), [what the relay does](../docs/experiments/2026-10-12-the-relay-follows-the-delivery-timeout-a-viewer-asks-for.md)).

`/publish?keyint=2000` makes a Group two seconds long (a second when left out), and `/watch?join=no` waits for the next key frame where the page otherwise fetches the Group it comes into ([record](../docs/experiments/2026-10-12-the-first-picture-of-a-viewer-who-comes-late-fetch-or-wait.md)).

**Two versions of the video (a trial)**: `/publish?ladder=1` encodes the picture a second time at 320x180 and 500 kbit/s and has both tracks in one alternate group of the catalog; `/watch?abr=1` plays the smaller one while the picture comes late and the larger one again when it has come in time for a while ([record](../docs/experiments/2026-10-12-two-versions-of-the-video-and-a-viewer-that-changes-between-them.md)). With `&judge=try` the page receives the larger one for 2 s before it shows it, and shows it only when its frames came in time ([record](../docs/experiments/2026-10-12-a-try-before-going-up-that-comes-again-as-soon.md)); `&versionTimeouts=A,B` gives each version its own delivery timeout. Without the two flags the page encodes and plays one version.

**A viewer that comes before the publisher** begins to play when the room says that a broadcast was registered, which the room tells everybody in it as it happens: the picture is there half a second after the publishing page was opened ([record](../docs/experiments/2026-10-13-a-page-that-waits-for-the-broadcast-at-the-relay.md)).

**A viewer that waits at the relay (a trial)**: `/watch?wait=1` does not go by the room's word that a broadcast has begun. It connects at once, its SUBSCRIBE for the catalog asks the relay to hold it for 30 s until a publisher is there, and when none came, and when the publisher has gone, it asks again on the session it has ([record](../docs/experiments/2026-10-13-a-page-that-waits-for-the-broadcast-at-the-relay.md)). Without the flag the page connects once the room says that a broadcast has begun.

**The chat goes over MoQ tracks** (decided on 2026-10-24): `mise run demo` starts the gatherer of the chat (`xmoq-chat`) beside the relay, with its store in `demo/.chat/chat.sqlite3` (not committed; `--chat-store FILE` for another), and each page is a participant over a connection of its own (`keepInTheRoom` of [`@xmoq/chat`](../packages/chat/README.md)), which connects again when the session ends, to the relay a GOAWAY names where one came. Every line shows the time the gatherer took it and its number in the room. A page that comes has the newest 16 lines by a Joining FETCH. When its subscription to the room is made again, or it connects again, it fetches the lines after the newest it shows, 200 at the most (`?chatCatchUpLines=N`); where it missed more, it fetches the newest 200 and one dimmed line above them says which lines were not fetched. A line longer than 512 bytes, or the sixth in a second, is not sent, and the line under the form says so (`--chat-text-bytes N` and `--chat-lines-per-second N` of `serve.ts` for other limits, which it gives the gatherer and the pages alike). Where the gatherer is not built, the pages have no chat ([records](../docs/experiments/2026-10-24-demo-chat-over-tracks.md), [and](../docs/experiments/2026-10-24-the-chat-off-the-rooms-websocket.md)). Two pages with the same name (two publishing pages are both `publisher`) announce the same namespace and publish the same track `said`; what the relay and the gatherer make of the two was not tried.

**Captions from the transcriber** (decided on 2026-10-25): `mise run demo` also starts the transcriber (`xmoq-transcriber`, a release build) beside the relay, where its models are in the cache (`mise run speech-model` and `mise run speech-model -- silero-v5.1.2`; without them the line under the picture says so, and the demo goes on without captions). It listens under `xmoq/broadcast/demo`, the broadcast of the room "demo", and the watching page shows each piece it reads under the picture, for 8 s or until the next comes (`keepFollowingTranscript` of [`@xmoq/transcript`](../packages/transcript/README.md), `showCaptions` of `@xmoq/transcript`, on a session of its own, which goes to the relay a GOAWAY names where one came). A page that comes shows the latest piece where it was read less than 8 s before. **Open `/publish?audio=mic` and speak**: the tone the publishing page sends otherwise is told to be no voice, and no caption comes. The words come about a second after the voice ends. It reads in the language each piece is in (`--transcribe-language auto`); `--transcribe-language ja` is twice as fast, and has an English voice come out in Japanese. With `--auth` the transcriber is given the relay's key, and the watching page asks the room for a `sub` token for `xmoq/transcript/xmoq/broadcast/demo` ([record](../docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md)).

**The chat with `--auth`**: each time a page connects for the chat, it asks the room for two tokens, `sub` for `xmoq/chat/<kind>/<room>` (`broadcast` on the publishing and the watching page, `meeting` on `/meet`), which its connection carries (SETUP, the SUBSCRIBE for `lines`, the Joining FETCH), and `pub` for `xmoq/chat/<kind>/<room>/<name>`, which only its PUBLISH_NAMESPACE carries (`publishNamespaceToken` of `joinChat`). The gatherer is given the relay's key (`--auth-key-hex`) and makes its own tokens. A gatherer built before it took that flag is not started with `--auth` (`serve.ts` asks the binary), and the pages then have no chat, as the line under the picture says ([record](../docs/experiments/2026-10-24-the-demo-chat-with-tokens.md)).

**A meeting** (`/meet`, decided on 2026-10-25): open <http://localhost:8080/meet> in two or more tabs. Each tab is a participant of the meeting of the room `?room=NAME` ("demo" when left out). It asks the room of the meeting for a seat (`/meetings/<room>/seats`) under a random id and keeps it by renewing it, publishes a small picture with its id on it and the microphone (`?audio=none` for no sound) under `xmoq/meet/<room>/<id>`, and shows the others with `createMeeting` of [`@xmoq/meeting`](../packages/meeting/README.md): the sound of all of them, which plays after a click on **sound**, and the small video of the first 8 who came. Past 49 the room gives no seat, and the tab says the meeting is full and tries no more. Publishing and receiving are two connections, one with the seat's `pub` and one with its `sub`. The transcriber listens under `xmoq/meet/demo` as well, and the room names `xmoq/meet/<room>` as the transcript of its meeting (`MEETING_TRANSCRIPT_PREFIX`) beside `xmoq/broadcast/<room>` as that of its broadcast (`TRANSCRIPT_PREFIX`), so an agent in the meeting (`mise run agent -- --room http://127.0.0.1:8787/meetings/demo --out DIR`) gets what each one says, and one in the broadcast what the publisher says. The meetings of other rooms are not transcribed ([record](../docs/experiments/2026-10-25-the-demo-has-a-meeting-page.md)). The page has the room's chat under its id, as the other pages have it. **Claude Code without the CLI** (Chromium alone): **link a folder for Claude Code** has the person choose `~/Documents/xmoq` once (make it first; Chromium lets a page have no Documents folder itself), and **call Claude Code** writes the meeting into `~/Documents/xmoq/<room>/<id>/` as `xmoq-agent --out` writes (`events.jsonl`, `frames/`), with the meeting's transcript, and does what is written into `inbox.txt` there: a line is said in the chat as the page, `/frame <id>` writes a picture; pressed again it stops. The page says which folder; tell Claude Code to read it, and to follow `events.jsonl` with `tail -F` (the page writes it anew each second). An experiment: what the others say is kept on the person's disk without their knowing, and Claude's lines are said under the person's name ([`/meeting`](../packages/meeting/README.md), [record](../docs/experiments/2026-10-25-a-page-writes-for-claude-code.md)).

`bun demo/check.ts` checks a running demo in headless Chromium: the frames a second, and that sound leaves the watching page.

Ctrl-C stops the web server, the relay, the gatherer and the transcriber. `bun demo/serve.ts --port N --relay-port M` chooses other ports.

The publishing page sends a second track with sound, Opus: a quiet tone that goes up and down (`?audio=mic` takes the microphone, `?audio=none` sends none). On the watching page, click "sound" to hear it: a browser lets a page make sound only after a click.

**The tracks are found in a catalog.** The publishing page puts out an MSF catalog (`createCatalogPublisher` of `@xmoq/msf`) with its video track (`avc3.42E01F`) and, unless `?audio=none`, its audio track (`opus`); the watching page reads it on every session (`followCatalog`) and plays the first track with the role `video` and the first with the role `audio`, by the names and codecs it finds there. Others can read it: `xmoq-moqt-client --catalog-video`, and `mlmsub -videoname video -audioname audio` of moqlivemock ([record](../docs/experiments/2026-10-12-the-demo-has-a-catalog.md)).

**Picture and sound are played together, by the times their Objects carry.** The playing is `createPlayout` of `@xmoq/media`; the page draws, counts and shows. The watching page holds each frame and each piece of audio for 150 ms, counted from the fastest one that came (`/watch?buffer=N` for another time, in milliseconds; 0 plays each as it comes, and then most of the audio is too late to be sounded). Every 2 seconds the publishing page shows a white square and sounds a beep at the same moment, for 200 ms; the lines under the picture say how far apart the two came out, and how long a frame and a piece took from the publisher's clock to this page.

**A room** comes with it: [`@xmoq/room`](../packages/room/README.md), run locally with Miniflare (its dependencies are the workspace's, which `mise run demo` installs), asked over HTTP. The publishing page publishes in the namespace its room names, `xmoq/broadcast/<room>`, and the watching page subscribes there and waits at the relay for a publisher (RENDEZVOUS_TIMEOUT), so the two may be opened in either order; `?room=NAME` on both pages is another room, and `?wait=no` has the watching page close its session where nobody publishes and connect again. The room is asked once, when the page opens, and again every second until it answers; a token it could not be asked for is none, and the connection that wanted it is refused by a relay with `--auth` and made again. **How many are there, and reactions, are not shown** for now: they went over the room's WebSocket, which is gone, and come back as tracks of the gatherer of the chat ([`crates/chat`](../crates/chat/README.md)).

**A session that ends** (the relay stopped, the link gone for more than 6 seconds): both pages say so in their first line and connect again after `DEFAULT_RECONNECT_DELAY_MS` of `@xmoq/moqt` (`keepPublishing` and `keepPlaying` of `@xmoq/media`).

**Tokens are asked for** (decided on 2026-10-26; who may get one is [undecided](../docs/open-questions.md)): `bun demo/serve.ts` makes a key for each run and gives it to the relay and to the rooms. The relay refuses a request without a token; the pages ask the room for one (`pub` to publish, `sub` to watch) and give it to their connection, which puts it into every request. The relay looks at the token in SETUP as well, and closes a session without one: `?token=no` on a page asks for none, and the page says that the relay closed the session as not authorized. `--no-auth` runs with no key, and the relay takes every request: for debugging and experiments.

The times are compared on one machine, where the two pages have the same clock.

## Every flag

Read off the code on 2026-10-13 (`demo/broadcast/watch.ts`, `demo/broadcast/publish.ts`, `demo/broadcast/room.ts`, `demo/lib/`, `demo/serve.ts`). Each is a query of the page's address unless it begins with `--`. "Trial" is something tried behind the flag and not the demo's way; the sections above have what each trial showed.

| Page | Flag | What it does | Without it |
|---|---|---|---|
| both | `room=NAME` | The room | `demo` |
| both | `via=PORT` | The relay through another port of this machine, where something stands in between (`spikes/lossy-link`) | The relay's own port |
| both | `token=no` | Asks the room for no token, to see the relay refuse | A token where the rooms give one |
| both | `uni=N` | How many streams of the peer are read at once | The library's |
| both | `timer=page`, `timer=slow` | The page's own timer, and that timer as a background tab has it | A worker's timer |
| both | `diag=1` | More in the page's report, for looking into a run | |
| meet | `room=NAME` | The room whose meeting the page is in; `uni=N`, `diag=1` and `timer=` as for the other two | `demo` |
| meet | `audio=none` | No sound is sent | The microphone |
| publish | `clockSkewPpm=N` | For measuring: the painted clock's times run N parts in a million faster than the machine's clock (the tone's do not follow: the encoder counts samples), as a camera's against a microphone's | 0 |
| publish | `toneSkewPpm=N` | For measuring: the tone makes its pieces N parts in a million faster than the machine's clock passes, as a microphone whose samples are counted by a clock of its own | 0 |
| publish | `canvasClock=performance` | For measuring: the painted clock is timed by `performance.now()` and the tone by `Date.now()`, two clocks that move apart by themselves (60 ms in an hour on one machine) | `Date.now()` for both |
| publish | `stamp=least` | Both encoders lay their source's times onto the wall clock by the least of the last minute (`clock: "least"` of `@xmoq/media`), which holds a source whose clock is not the wall's | The wall clock is read with the first frame only |
| watch | `clockBehindMs=N` | For measuring: the page's clock is N ms behind for its first 5 seconds, which puts the fastest way N ms too fast until those seconds are forgotten, a minute later | 0 |
| watch | `clockSkewPpm=N` | For measuring: the page's clock runs N parts in a million faster, as a viewer's on another machine against its publisher's | 0 |
| publish | `size=360`, `720`, `1080` | The picture's size | 360 |
| publish | `bitrate=N` | Bits a second of the video | The size's: 1.5, 3 and 6 Mbit/s |
| publish | `keyint=MS` | A key frame every so many milliseconds | 1000 |
| publish | `picture=busy` | A picture that fills the bitrate it asks for | The clock alone |
| publish | `source=camera` | The camera | The painted clock |
| publish | `audio=mic`, `audio=none` | The microphone; no sound | A tone, Opus, mono |
| publish | `audioGroup=N` | Pieces of sound in a Group, and so on a stream | 1 |
| publish | `audioPriority=N` | The Publisher Priority of the sound's streams, 0 the highest | None named (128, as the video) |
| publish | `mapping=stream-per-object` | Every Object of the video on a stream of its own | A stream for a Group |
| publish | `ladder=1` | Trial: a second, small version of the video | One version |
| publish | `dynamic=MS` | Trial: the video has dynamic Groups, the shortest MS long; a viewer that asks for a new Group gets one | No dynamic Groups |
| watch | `buffer=N`, `buffer=auto` | The playout buffer in milliseconds; `auto` (trial) follows how late Objects come | 150 (`DEFAULT_PLAYOUT_BUFFER_MS` of `@xmoq/media`) |
| watch | `autoMax=MS`, `autoShare=F` | With `buffer=auto`: its limit, and the share of the Objects it covers | 400, 0.99 |
| watch | `autoDown=quarter`, `autoWindow=MS` | Trials, with `buffer=auto`: it comes down a quarter of the way at a time, and looks back so far | 10 ms at a time; 10000 |
| watch | `audioMin=MS` | The least the sound waits where the buffer is shorter | 30 |
| watch | `order=no` | A piece of sound that comes before its turn is not held for the ones before it | Held |
| watch | `join=no` | No Joining FETCH for the video: the picture begins with the next key frame | A Joining FETCH of the Group under way |
| watch | `audioJoin=1` | A Joining FETCH for the sound as well | None |
| watch | `newgroup=1` | Off when left out, decided (the page fetches the Group it comes into): the video's SUBSCRIBE asks the sender for a new Group (NEW_GROUP_REQUEST 0); with `join=no`, the picture begins with the key frame it makes for that | Not asked |
| watch | `video=no` | The sound alone | Picture and sound |
| watch | `videoTimeout=MS` | The delivery timeout asked for the video; 0 asks for none | 150 |
| watch | `audioTimeout=MS` | The same for the sound | None asked |
| watch | `cutSound=S` | For trying: the first subscription of the sound is cancelled after `S` seconds; the page subscribes to the sound again on the session, and the picture goes on ([record](../docs/experiments/2026-10-14-the-sound-subscribed-to-again-alone.md)) | Not cancelled |
| watch | `videoTimeoutThen=MS`, `videoTimeoutAfter=S` | Changes the video's timeout while it plays, with a REQUEST_UPDATE, after S seconds | No change; 10 |
| watch | `abr=1` | Trial: changes between the versions of `ladder=1` | The first version |
| watch | `judge=try` | Trial, with `abr=1`: a higher version is received for 2 s before it is shown | Shown at once |
| watch | `versionTimeouts=A,B` | Trial, with `abr=1`: a delivery timeout for each version | `videoTimeout` for each |
| watch | `wait=no` | Closes the session where nobody publishes, and connects again after the pause | Waits for the broadcast at the relay, on one session (RENDEZVOUS_TIMEOUT) |
| both | `chatCatchUpLines=N` | How many of the lines it missed the page fetches at the most when it subscribes or connects again: 1 to 1,000, the lines one FETCH of the gatherer gets; the page shows the newest 200 of them | 200: what the page shows |

`bun demo/serve.ts` (`mise run demo`): its flags, each with what it does and what it is without it, are the options of `parseArgs` at the top of `demo/serve.ts`. **The values the applications choose are its flags too**, so that a run can choose others: the limits of a line of the chat (`--chat-text-bytes`, `--chat-lines-per-second`), whose video a page of the meeting receives (`--meeting-most-watched`), and the transcriber's models (`--speech-model`, `--vad-model`). What the meeting's room says is not a flag: a page of the meeting keeps as many others as the room has seats, less itself.

