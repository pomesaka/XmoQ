# The gatherer of the chat

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, chat over MoQ tracks (decided on 2026-10-24); the history in SQLite and the limits of the chat, both decided on 2026-10-24.

## Objective

Chat moved onto MoQ tracks (decided on 2026-10-24): each participant announces `xmoq/chat/<room>/<name>` and says a line as one Group on its track `said`. Nothing gave the lines of a room one order, or kept them once the relay's cache had let go ([the run of 2026-10-12](2026-10-12-chat-as-moq-tracks-beside-the-rooms-websocket.md)). `xmoq-chat` (`crates/chat`) is a client of the relay that is told of every participant by one `SUBSCRIBE_NAMESPACE` for `xmoq/chat` (§6.1), subscribes to each `said`, numbers the lines of a room in the order they reach it, keeps them in SQLite, and publishes them on `lines` of `xmoq/chat/<room>`, answering a FETCH for older lines from the store. Does it, through the own relay, with own-rust clients as the participants and the viewers?

## Expected

**Written before the first run** (it is the head of `interop/chat.ts`):

- The gatherer is told of every participant and subscribes to each one's `said` (`SUBSCRIBED`).
- Lines of `a` and `b`, one after the other, are numbered 0 to 3 in the order they reach the gatherer, and a viewer subscribed to `lines` gets them live: Groups 0 to 3, whose JSON names a, b, a, b.
- `c` says a line of 513 bytes: dropped as `too-long` (the limit is 512).
- `d` says 8 lines at once: 5 are taken (4 to 8) and 3 dropped as `too-fast` (the limit is 5 in any second).
- A viewer that comes then FETCHes 0 to 100 and gets Groups 0 to 8 from the store, with `FETCH_OK` ending at {8, 1} (§10.13).
- The gatherer is stopped and started again on the same store, with `--fetch-lines 3`: it publishes the room with `LARGEST_OBJECT` 8, follows the participants that are still there, and the line of a new participant `e` is numbered 9.
- A viewer then FETCHes 0 to 100 and gets an End of Unknown Range indicator up to Group 6, then Groups 7 to 9, and `FETCH_OK` ends at {9, 1}.

## Environment

macOS 14.4, arm64, loopback; the working copy at `ppqsotnwmptz` (on `main` at `d6f3e406`) with this change; rusqlite 0.40.2 with SQLite bundled (libsqlite3-sys 0.38.2) built by Apple clang; `bun interop/chat.ts --relay own|moqtail --port 45390`; `mise run check`.

## Result

**Through the own relay, as expected**, in the first run (`bun interop/chat.ts --relay own --port 45390`) and in each of six runs of `bun interop/matrix.ts --only chat: --base-port 45300` after it, the last three with the code as it is now:

| Step | The gatherer printed | The viewer |
|---|---|---|
| a, b, a, b say a line each | `LINE room="r" number=0 name="a"` to `number=3 name="b"` | live: Groups 0 to 3, names a, b, a, b |
| c says 513 bytes | `LINE_DROPPED room="r" name="c" why=too-long dropped=1` | |
| d says 8 lines at once | `LINE` 4 to 8, then `LINE_DROPPED ... why=too-fast` three times | |
| A late viewer FETCHes 0 to 100 | `FETCH_ANSWERED room="r" start=0,0 end=8,1 lines=9 unknown_through=none` | `FETCH_OK end_group=8 end_object=1`, Groups 0 to 8, FIN |
| Stopped (SIGTERM), started on the same store with `--fetch-lines 3` | `STORE ... rooms=1`, `ROOM_PUBLISHED room="r" forward=true largest=8`, `SUBSCRIBED` for a to d | |
| e says a line | `LINE room="r" number=9 name="e"` | |
| A viewer FETCHes 0 to 100 | `FETCH_ANSWERED ... end=9,1 lines=3 unknown_through=6` | `FETCH_OK end_group=9 end_object=1`, `FETCH_RANGE_END kind=unknown group=6 object=0`, Groups 7 to 9 |

When the first gatherer left, the relay cancelled its subscriptions to the participants (`SUBSCRIPTION_CANCELLED`), and asked them again for the second (`SERVING track_alias=1`).

**Through `moqtail`'s relay (0.15.0), not as expected at the last step.** Everything up to it went as through the own relay. Of the answer the gatherer cut to 3 lines, the viewer got `FETCH_OK end_group=9 end_object=1` and then Groups 0, 1 and 2 (the lines numbered 7 to 9: names d, d, e), and no indicator. moqtail's decoder takes no prior Location from an End of Range indicator (`FetchObject::context` of `libs/moqtail-rs/src/model/data/fetch_object.rs` at `dd8623d`: "they MUST NOT update prior state"), where §11.4.4.2 has "Prior Group ID and prior Object ID: The values from the End of Range indicator", so the first Object's Group ID Delta 0 read as Group 0; and its relay passes on the Objects of an upstream fetch stream without the indicators (`fetch_handler.rs`). Ten runs ended so (five of `chat.ts` alone, five in the matrix, the last three with the code as it is now), and the pairing is in the matrix as expected to fail with that line. **In one more run through moqtail's relay**, its first in the matrix, the second gatherer's PUBLISH and its four SUBSCRIBEs got no answer within 10 s: the relay logged `Received Publish message for track: lines` and `Subscriber ... added to 1 publisher(s)` for each `said`, and nothing after; the participants were not asked again. Not seen in the nine runs after it, and not looked into.

`mise run check` passed (the last lines are in the report of this change), with the table tests of the library (the namespaces, the payloads, the rate, the numbering, the plan of a FETCH, the flags, what becomes of a line) and of the store (kept and opened again, a range, a number kept once).

**Not run**: a GOAWAY of the relay; a relay that is stopped while the gatherer runs (it connects again a second later, read from the code); a Descending FETCH; a FETCH through the relay that is answered from the relay's cache; more than five participants or more than one room; a participant whose subscription is refused; the limits against moqtail's relay beyond what the run has; anything on Linux, where CI builds SQLite with its `gcc`.

After that, `mise run interop -- --base-port 45300` ended 189 of 189 as expected (303 s). Then the names, the payloads and the limits of a line were moved into `xmoq-chat-tracks`, with the reader of `lines` and the writer of `said` added and the shared tables of `fixtures/chat-tracks/` read in its tests. After the move, `mise run check` failed once in the Rust checks, whose lines were not kept, and passed twice after it with nothing changed; `bun interop/matrix.ts --only chat: --base-port 45300` ended 2 of 2 as expected. The whole matrix was not run again after the move.

## After the review

The review of the change found ten faults, and the user decided on a gatherer for each room (2026-10-24). What changed: a room is numbered and published while somebody is in it, and not from the store at the start; its PUBLISH ends with `PUBLISH_DONE` `TRACK_ENDED` once every stream of its lines is closed, with their number (§10.11); the same order for a `PUBLISH_DONE` after a refused `REQUEST_UPDATE`; `said` is subscribed to with the Largest Object filter (§5.1.2); the relay's direction of a FETCH's request stream is stopped at the gatherer's FIN where the relay has not ended it; answers to `REQUEST_UPDATE` are never dropped, and a request whose queue of 64 is full is cancelled with `EXCESSIVE_LOAD`; a `SUBSCRIBE_NAMESPACE` that ends is asked for again alone, and sessions are connected again, after a wait that doubles from 1 s to 30 s; the store runs with `synchronous = FULL` and `locking_mode = EXCLUSIVE`, one gatherer to a file; the FETCH answers being written hold 64 MiB of payload at the most; `--room` (up to 8).

**Expected, written before these runs** (at the head of `interop/chat.ts`): as before through the own relay, and the second gatherer, with `--room r`, watches `xmoq/chat/r` and never asks a participant of room s for its track; when everybody has left, `ROOM_DONE room="r" status=0x2 streams=1`, and a FETCH for the room is refused with `DOES_NOT_EXIST` (0x10). Through moqtail's relay, with no restart and `--fetch-lines 6`, the late viewer's FETCH is to show an End of Unknown Range up to Group 2 and Groups 3 to 8, which moqtail's relay is expected not to pass on.

**Why the run through moqtail's relay has no restart now**: CI failed it once (191 of 192), as one run here had, with the second gatherer's PUBLISH and SUBSCRIBEs unanswered. moqtail's track manager (`apps/relay/src/server/track_manager.rs` at relay@0.15.0, read with `gh api` on 2026-10-24) takes its locks in both orders: `remove_track` holds `tracks` for writing and then takes `track_aliases` for writing; `get_track_by_alias` holds `track_aliases` for reading and then takes `tracks` for reading. A removal of the first gatherer's tracks at the moment the second's PUBLISH looks up an alias would wedge the track manager, which fits what was seen (`Received Publish message` and nothing after, from the relay's every handler). This was read, not shown with a trace. Nothing on the gatherer's side, short of waiting an unknown time after the restart, avoids it, so the restart runs through the own relay alone.

| Run | Result |
|---|---|
| `bun interop/matrix.ts --only chat: --base-port 45300`, the last with the code as it is now | 2 of 2 as expected (1.5 s) |
| `bun interop/matrix.ts --only "own relay: chat:" --base-port 45300`, 5 times | 5 of 5 as expected |
| `bun interop/matrix.ts --only "moqtail relay: chat:" --base-port 45300`, 15 times | 15 of 15 as expected (`told of unknown lines up to: none, not 2`) |

Through the own relay the second gatherer printed `WATCHING prefix=xmoq/chat/r`, numbered e's line 9, and, once the five participants left, `PARTICIPANT_GONE` five times and `ROOM_DONE room="r" status=0x2 streams=1`; the participant of room s printed no `SERVING`, and the FETCH after was `FETCH_REFUSED ... error_code=0x10`. No `FETCH_REQUEST_STOPPED`: the own relay ended its direction of each FETCH itself.

**Not run**: a `REQUEST_UPDATE` the gatherer refuses (its `PUBLISH_DONE` after the streams of the room close is read from the code); a full queue of a request; a watch that ends and is asked for again; a relay that ends sessions at once (the growing wait); a FETCH over 64 MiB; two gatherers on one store against each other (a test of the store has a second opening of the file refused); a power loss; the cost of `synchronous = FULL`; ten gatherers of ten rooms.

Then, on `main` with the change of the relay's request writer (#270) under it, `mise run check` passed and `mise run interop -- --base-port 45300` ended 192 of 192 as expected (344 s).
