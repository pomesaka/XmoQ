# A reader of the room for an agent

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, chat; the reader for agents (`crates/agent`).

## Objective

`xmoq-agent` is to let an agent read a room: the chat, who comes and leaves, as lines of JSON on stdout and in a file, and to say lines of its stdin in the chat. Does it, through the own relay and the real gatherer, with a page as the other participant?

## Expected

Written before the first run, at the top of `interop/agent.ts`:

- The page joined before the reader came: the reader's SUBSCRIBE_NAMESPACE for `xmoq/chat/r` is told of `xmoq/chat/r/p`, and it writes `{"type":"joined","name":"p"}`; nothing joins for its own name `claude`.
- The page says `hello from p`: the reader writes a `chat` line from p with that text and the number the gatherer gave it (0).
- A line of the reader's stdin, `hello from claude`, reaches the page as a line of `claude` (1), and the reader writes it as well.
- `//slash` is said as `/slash` (2); `/frame` and `/nothing` say nothing and are written as `error` lines.
- The page leaves the chat: `{"type":"left","name":"p"}`.
- `events.jsonl` has the same lines as stdout, in the same order.

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless; the own relay, `xmoq-chat` and `xmoq-agent` of this tree (debug builds). First on `main` at `bdad71ce` with this change, before `/frame` took pictures: `bun interop/agent.ts --port 33100`. Then on `main` at `490b0b21` (`xmoq-picture` in) with this change: `bun interop/agent.ts --port 33100` with `--frames off` and the error it gives, a one-off script for `/frame`, and `mise run interop -- --base-port 33000` with the machine's lock.

## Result

**As expected, in both runs alone.**

- The relay told the reader of `claude` and `p` (`NAMESPACE suffix=claude`, `NAMESPACE suffix=p`); the reader wrote `joined` for p alone.
- The lines written, without `at`: `broadcast` (began), `joined` p, `chat` 0 from p, `chat` 1 and 2 from claude (`hello from claude`, `/slash`), `error` (`frame: not built yet` in the first run, `frame: --frames off` in the second), `error` (`no command /nothing: ...`), `left` p. The page had `1 claude hello from claude` and `2 claude /slash`.
- `events.jsonl` had the same 8 lines.

`/frame`, once, by a script that was not kept: a page published its broadcast (`publishBroadcast`, 640x360), the reader read its catalog and subscribed to `video`, and 2.5 s after it came `/frame` was answered by a `frame` line of Group `1792884901959`, Object 12, 640x360, whose file was a PNG of 1,634 bytes with the page's picture (a grey field and a green square). `Frames` counted 81 Objects given, 13 decoded, 62 passed over (those of the Groups before), 1 taken.

The full run ended 219 of 219 as expected (441.3 s), the reader's pairing among them (1.0 s).

### One `Frames` for the whole run

Then the reader kept one `Frames` for the run and called `begin_again` on each session's video (`Frames::begin_again` of #329), and its files lost the prefix of their session. Run once more by a script that was not kept, with `--frames-kept 1` and two `/frame` 1.5 s apart: `000001-g1792887291715-o13.png` (1,602 bytes) and then `000002-g1792887291717-o2.png` (2,156 bytes), and the first was removed when the second was written. `Frames` counted 99 given, 17 decoded, 79 passed over, 2 taken. The full run after it ended 225 of 225 as expected (468.1 s).

## What was not run

- A room (`--room`): the interop runs have none, and the run uses `--url` with `--chat-room`. The room's tokens are asked for by the code `xmoq-watch` uses (`RoomSocket::chat_tokens`, now in `xmoq-viewer-io`), which the reader's run does not reach.
- A reader that connects again, and the 2 s after which those not told of again are written as `left`: the table test of `presence` alone.
- `/frame` in `mise run interop`: `anna-4aa4` adds that pairing. `--frame-every-ms`, and a `/frame` that gets no picture within 10 s.
- A second session with video through a relay, where `begin_again` is called on a `Frames` that had pictures: the table tests of `xmoq-picture` (`src/tests.rs`) alone, and the PNG pairing of `anna-4aa4` when it reconnects.
- `speech`: not built yet.
