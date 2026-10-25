# An agent reads the demo with xmoq-agent

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4; the reader for agents (`crates/agent`).

## Objective

Can an agent, by the commands at the top of `crates/agent/README.md`, start a demo of its own beside the one a person runs, put `xmoq-agent` in its room with `mise run agent`, say a line, and have a picture of the broadcast?

## Expected

Written before the run:

- `bun demo/serve.ts --relay-port 33443 --port 33080 --rooms-port 33787 --chat-store <file>` starts the relay, the rooms, the gatherer and the pages on those ports, and nothing on 4443, 8080 or 8787 is touched.
- With the publishing page open (in a headless Chromium, which a person would open by hand), `mise run agent -- --room http://127.0.0.1:33787/rooms/demo --name claude --out <dir> < <pipe>` writes `broadcast` `began` for `xmoq/demo`.
- `hello from the agent` written to the pipe comes back as a `chat` line from `claude`; `/frame` is answered with a `frame` line whose PNG is the page's picture at about that moment.

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless, for the publishing page alone; the relay and the gatherer (debug builds) and `xmoq-agent` (the release build `mise run agent` makes) of this tree, on `main` at `3da4dddd` with this change. No transcriber: the demo starts none. The commands were those of the README with a directory of the session in place of `/tmp/room`, and a pipe held open by a `sleep` as there.

## Result

**As expected.**

- The demo printed `publish: http://localhost:33080/publish`, `rooms: http://127.0.0.1:33787/` and the chat's store.
- `mise run agent` built the release binary (8.2 s) and the reader wrote, in this order: `{"name":"publisher","type":"joined"}` (on its session at the room's relay, before the broadcast was registered), `{"namespace":["xmoq","demo"],"state":"began","type":"broadcast"}`, `{"from":"claude","number":0,...,"text":"hello from the agent","type":"chat"}`, and `{"asked":"command",...,"height":360,"path":".../frames/000001-g1792894060961-o2.png","timestamp_us":1792894082626000,"type":"frame","width":640}`, 3 ms after the chat line.
- The PNG showed the demo's painted clock at `02:08:02.622` (UTC) and `frame 653`; the line's `timestamp_us` is `02:08:02.626`.
- At the end, `XMOQ_AGENT_FRAMES given=1317 decoded=3 passed_over=1298 taken=1`. The SUBSCRIBE to the transcript (`speech` of `xmoq/transcript/xmoq/demo`, which nobody published) was refused with `0x2` as the relay stopped.
- Every process of the run was stopped by its PID; 4443, 8080 and 8787 were not touched.

### With the transcript

Once the demo started its transcriber (#345), the same again on `main` at `0cb80195`, before `mise run agent` was on `main`: the reader by what that task runs (`cargo run --release --locked -p xmoq-agent -- ...`). Written before the run: the reader writes a `speech` line from `["xmoq","demo"]` with the words of the voice the page sends, beside the chat and the picture.

- `bun demo/serve.ts` failed first, bundling the page (`Could not resolve: "@xmoq/json"`): a package new on `main`, which `bun install --frozen-lockfile` put in. `mise run demo` installs; the README's commands now do too.
- With `--transcribe-language en` the demo said `captions: the transcriber reads xmoq/demo in en; open /publish?audio=mic to be heard`. The page was opened at `/publish?audio=mic` in a headless Chromium given `fixtures/speech/hello-from-the-relay.wav` as its microphone (`--use-fake-ui-for-media-stream --use-fake-device-for-media-stream --use-file-for-fake-audio-capture=...`).
- **As expected.** The reader wrote `broadcast` `began`, `joined` `publisher`, then `{"from":["xmoq","demo"],...,"text":"hello from the relay","type":"speech"}` 1 ms after the transcriber had read it (`read_at`), then the chat line and a `frame` (640x360, the painted clock `02:21:35.348`, `timestamp_us` `02:21:35.351`).
- The piece lasted 15 s (`start_us` to `end_us`): the fake microphone plays the file of 1.2 s over and over without a pause, and the transcriber cuts a piece at a pause.
- Without `?audio=mic` the page sends a tone, in which the transcriber hears no words; the README's commands now open the page with it.

## What was not run

- A transcript with `--transcribe-language` left to `auto`, and of a person's voice through a real microphone.
- The commands with `--auth`.
