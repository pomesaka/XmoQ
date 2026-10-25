# A reader joins a room that starts later

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4; the reader for agents (`crates/agent`) and the terminal viewer (`crates/watch`).

## Objective

In the PM's first `mise run agent-demo -- start` (#353), the reader ended at once with `the room at ws://127.0.0.1:30787/rooms/demo/ws?name=claude: IO error: Connection refused`: the demo had printed where the rooms are, and workerd did not listen yet. Two changes: `RoomSocket::join_when_there` of `xmoq-viewer-io` asks a room that refuses the connection again, every 1 s by `xmoq-again`, for 30 s (`ROOM_COMES_WITHIN`), and `xmoq-agent` and `xmoq-watch` join by it; and `agent-demo.sh start` waits for the rooms' port before it starts the reader.

## Expected

Written before the runs: a reader started 5 s before the demo writes no `error` and, once the page publishes, `joined` and `broadcast` `began`; `start` of the script returns as before.

## Environment

macOS 14.4, arm64; `main` at `ef51bfda` with this change; `xmoq-agent` a release build; the ports 33443, 33080 and 33787.

## Result

**As expected.**

- The tests of `room_socket.rs`: a room that begins listening 1.5 s after the first try is joined; a refusal for longer than `within` (0 s) ends with `refused for 0 s`; a connection that is accepted and closed without a word is not asked again.
- `xmoq-agent --room http://127.0.0.1:33787/rooms/demo --name early` began at 12:01:55, with nothing on 33787; `mise run agent-demo -- start` began its demo at 12:02:00 (after its builds). The early reader wrote `joined` `claude` (the script's reader), `joined` `publisher` and `broadcast` `began`, and no `error`; `ended` when the demo stopped.
- `start` returned with the commands as before, and `stop` left none of the four nor their children (`ps -p` and `pgrep -f` on the ports found nothing).

## What was not run

- A room that comes after more than 30 s, and `xmoq-watch` against a room that comes later (it joins by the same function).
- The first `start` on a machine where workerd takes longer to listen than here, which is what failed in the PM's run.
