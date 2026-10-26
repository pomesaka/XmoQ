# The demo has a meeting page

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, meetings; the reader for agents.

## Objective

The demo had no page of a meeting. `/meet` is one participant: it takes a seat of the room (`join-meeting`) and publishes its picture and voice under `xmoq/meet/<room>/<id>`. It receives the others with `createMeeting` of `@xmoq/meeting`. The transcriber of the demo listens under `xmoq/meet/demo` as well, and the room names `xmoq/meet/<room>` as its transcript. Do three pages meet, and does an agent in the room's meeting see them and get what they say?

## Expected

Written before the runs:

- Each of 3 pages of `/meet` in one headless Chromium, whose fake microphone plays `fixtures/speech/hello-from-the-relay.wav`, is seated and publishes. Within 30 s it draws frames of the 2 others and receives their sound.
- `mise run agent -- --room http://127.0.0.1:33787/rooms/demo --meeting demo` does three things:
  - It writes the 3 as `participant` `joined`.
  - `/frame <id>` of the first page gives a `frame` line of that id.
  - Within 60 s it writes a `speech` line from `["xmoq","meet","demo",<id>]` of a page, with "relay" in it.
- 45 more seats are asked for over the room's socket: 3 pages, the agent and 45 make 49. A 4th page then says "the meeting is full: it takes 49".
- `mise run agent-demo -- start`: its reader, now given `--transcript xmoq/demo`, writes a `speech` line from `["xmoq","demo"]`.

## Environment

macOS 14.4, arm64, loopback. Bun 1.3.12 and Chromium 156.0.8078.4 of playwright-core, headless. `bun demo/serve.ts --relay-port 33443 --port 33080 --rooms-port 33787 --transcribe-language en`, without `--auth`. The relay and the gatherer are debug builds and the transcriber and the agent release builds, of `main` at `5e789920` with this change, with `ggml-large-v3-turbo-q5_0.bin`. The driver was a script that was not kept.

## Result

**As expected**, on the third run, after two fixes. The first run found that the catalog's sound track had no `bitrate`, which `@xmoq/msf` takes as needed. The second found that the page wrote its counts only on a change of who was there. Then:

```
page 0: {"seat":"seated","publishing":"publishing","receiving":"attending","others":{"1e3e…":{"frames":9,"audio":30},"4fb6…":{"frames":8,"audio":26}},"failed":[]}
{"id":"9eb088251e5d38c8","state":"joined","type":"participant"}   (and the 2 others)
{"asked":"command",…,"height":180,"id":"9eb088251e5d38c8",…,"type":"frame","width":320}
{"from":["xmoq","meet","demo","9eb088251e5d38c8"],…,"text":"Hello from the relay.","type":"speech"}
the 4th page: the meeting is full: it takes 49
the agent's error or refused lines: []
```

After a page that finds the meeting full stopped publishing as well, the same run passed again. The counts are those of the moment each page first had both others. The agent demo's reader wrote `{"from":["xmoq","demo"],…,"text":"Hello from the relay.","type":"speech"}`, and `stop` ended its five processes.

## What was not run

- `--auth`: the seat's tokens at a relay that asks for them were run without the demo ([record](2026-10-25-an-agent-takes-a-seat-in-a-rooms-meeting.md)).
- Sound heard by a person: the pages were headless and nobody clicked **sound**.
- More than 3 pages publishing, and a meeting of a room other than `demo`, which is not transcribed.
