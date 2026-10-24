# xmoq-chat

The gatherer of the chat. A client of the relay that is told of every participant of every room, takes the lines each one says, numbers the lines of a room in the order they reach it, keeps them in a SQLite file, and publishes them as one track a room. A viewer gets the lines live by SUBSCRIBE and the older ones by FETCH, which the relay passes to the gatherer.

```sh
mise run chat -- --url https://127.0.0.1:4443/ --cert-sha256 <64 hex digits> --store chat.sqlite3
mise run chat -- --url ... --cert-sha256 ... --store room-r.sqlite3 --room r     # one room alone
```

It runs until Ctrl-C or SIGTERM, and connects again after a session ends: a second later at first, twice as long after each session that lasted less than a minute, up to 30 seconds (`backoff` of [`xmoq-track-serving`](../track-serving/README.md), provisional). With a flag it does not know it prints its flags (`USAGE` in `src/args.rs`).

**Building it needs a C compiler**: `rusqlite` builds SQLite from the source it carries (`bundled`), through the `cc` crate. On macOS the Command Line Tools' `clang`, on Linux `gcc`.

## The tracks (decided on 2026-10-24)

| Who | Announces | Track | One Group | Payload of its Object 0 |
|---|---|---|---|---|
| A participant | `PUBLISH_NAMESPACE` `xmoq/chat/<room>/<name>` | `said` | one line | `{"text": string, "sentAt": number}` |
| The gatherer | PUBLISH (why, at the top of `src/bin/xmoq-chat/session.rs`) | `lines` of `xmoq/chat/<room>` | one line; the Group ID is its number in the room | `{"name": string, "text": string, "sentAt": number, "at": number}`, in that order |

`sentAt` is the sender's clock and `at` the gatherer's when it took the line, both milliseconds since the Unix epoch. `name` is the fourth field of the sender's namespace. The gatherer passes members of `said` it does not know over.

The gatherer asks with one `SUBSCRIBE_NAMESPACE` for `xmoq/chat`, or, with `--room`, one for `xmoq/chat/<room>` for each room given, and follows every namespace of exactly four fields `xmoq`, `chat`, room, name, the last two UTF-8; any other is passed over. Where such a request ends (the relay ends it, or tells more namespaces than the session keeps), it is asked for again alone, after the same growing wait, and the session goes on. It subscribes to each participant's `said` with the Largest Object filter (§5.1.2), so that a relay that still has older lines does not pass them again to a gatherer that connected anew; why, at `subscribe` in `src/bin/xmoq-chat/session.rs`. `NAMESPACE_DONE` ends the subscription to that participant.

**A participant whose subscription ends or is refused while its namespace stays announced is subscribed to again** (`ASKING_AGAIN ... in_ms=N`), by the rule every client of this repository asks by ([`xmoq-again`](../again/README.md), the watching page's): after 1 second (`--ask-again-ms`) where it ended or the refusal names no time, after the Retry Interval where it names one (no less than 100 ms, doubling while refusals follow each other, up to 5 s; no more than 30 s), also after a Retry Interval of 0. Its next lines are taken; what it said meanwhile is not fetched (decided on 2026-10-24). After `--ask-again-times` times in a row without a subscription that was established (30), it is not asked again until its NAMESPACE comes again (`NOT_ASKING_AGAIN`), nor after a track that was malformed. Why, at the top of `src/said_again.rs`.

**Rooms**: a room is numbered and published while somebody is followed in it. When the first participant comes, the room is numbered on from what the store holds, and published with its largest number as `LARGEST_OBJECT`. When the last one leaves, its PUBLISH ends with `PUBLISH_DONE` `TRACK_ENDED`, sent once every stream of a line of the room is closed (§10.11), with their number (`ROOM_DONE ... streams=N`). **A FETCH for a room nobody is in** reaches no publisher: the own relay refuses it with `DOES_NOT_EXIST`. Its lines are in the store, and a FETCH gets them again once somebody is in the room. Why not every room of the store, at the top of `src/rooms.rs` and of `session.rs`.

**Order**: the lines of a room are numbered from 0, without a gap, in the order they reach the gatherer; `sentAt` plays no part in it.

**A line is dropped and counted** (`LINE_DROPPED ... why=<word> dropped=<count>`) when its payload is not of the form above (`not-an-object`, `text`, `sent-at`: `sentAt` that is no integer from 0 to 2^53 - 1), when its text is too long (`too-long`), when its sender is over the rate (`too-fast`), and when it cannot be kept (`store`). Why a malformed `sentAt` drops the line, at `Dropped::Malformed` in `src/gather.rs`.

## Limits

The limits of the chat (Q43, decided on 2026-10-24), which flags change:

| Flag | Default | What it bounds | The flag's range |
|---|---|---|---|
| `--text-bytes` | 512 | Bytes of UTF-8 in the text of one line | 1 to 8,192 |
| `--lines-per-second` | 5 | Lines one person may say in any second; the line over it is dropped, and does not count | 1 to 1,000 |
| `--fetch-lines` | 1,000 | Lines one FETCH gets: of a longer range, the latest | 1 to 100,000 |

When a participant whose subscription ended is asked again, which flags change (**provisional**):

| Flag | Default | What it sets | The flag's range |
|---|---|---|---|
| `--ask-again-times` | 30 | Times in a row a participant is asked again without a subscription that was established; against the own relay, which refuses a `said` nobody publishes with a Retry Interval of 1,001, about two minutes | 0 (never) to 10,000 |
| `--ask-again-ms` | 1,000 | The pause after an end, and after a refusal that names no time or says 0 | 100 to 60,000 |

`--room <room>` (repeatable, up to 8) has the gatherer take, publish and answer FETCH for those rooms alone (Q48, decided on 2026-10-24: a gatherer for each room). More than 8 are refused at the start: a session at the own relay may have 8 `SUBSCRIBE_NAMESPACE` requests (Q45, decided on 2026-10-24), and a ninth would be refused for good while the gatherer looked as if it ran.

**Ten rooms of a hundred people** (Q41) are reached with a gatherer for each room, each with `--room` and a store of its own: each follows 100 participants, under the about 250 the own relay lets one session have (`MAX_PEER_REQUESTS`, 256 requests, the `SUBSCRIBE_NAMESPACE` and the PUBLISH among them). **Not measured.**

**A FETCH of more lines than `--fetch-lines`** gets the latest lines of its range and, for the lines before them, an End of Unknown Range indicator (§11.4.4.2), before the lines in Ascending order and after them in Descending order. `FETCH_OK` keeps the End Location that was asked for (§10.13). Why not a shorter End Location, no indicator, or a refusal: at the top of `crates/track-serving/src/fetch.rs`.

What the gatherer holds of what the relay and the participants send has a bound that no flag removes. **All are provisional.**

| What | Bound | Where |
|---|---|---|
| One stream of `said`, undecoded | 64 KiB; the stream is stopped and its line lost | `PAYLOAD_MOST`, `src/args.rs` |
| Participants followed | 4,096; one more is passed over (`PARTICIPANT_IGNORED`) | `MAX_PEOPLE`, `src/gather.rs` |
| Rooms numbered | 1,024; a participant of one more is passed over | `MAX_ROOMS`, `src/rooms.rs` |
| Objects that come before their `SUBSCRIBE_OK` | 64; the oldest is dropped | `session.rs` |
| Lines that wait for the `PUBLISH_OK` of their room | 64 a room; the rest are not sent live | `session.rs` |
| Subgroup streams of lines being written | 256; the rest are not sent live (`LINE_NOT_SENT`) | `session.rs` |
| FETCH requests answered at once | 32; one more is refused with `EXCESSIVE_LOAD` | `session.rs` |
| Bytes of payload the FETCH answers being written hold together | 64 MiB; a FETCH whose answer would go past it is refused with `EXCESSIVE_LOAD`. The lines are read one by one, and the reading stops at the bound | `MAX_BYTES_HELD`, `session.rs` |
| Messages for one request stream not written yet, the answers to `REQUEST_UPDATE` among them | 64; none is dropped (§10.9: "exactly one"): one more cancels the request with `EXCESSIVE_LOAD` | `COMMAND_QUEUE`, `streams` of `xmoq-track-serving` |
| A line's subgroup stream | 1 s for the relay to allow it, 5 s for the relay to take it; then it is reset, and the line is not sent live | `streams` of `xmoq-track-serving` |
| The relay's direction of a FETCH's request stream | Stopped with `CANCELLED` when the gatherer finishes its own, 3 s after the end of the fetch stream at the most, where the relay has not ended it | `session.rs`, `fetch_over` |
| The relay's SETUP; a stream for a request; the answer to a request | 10 s each | `session.rs` |

A line that is not sent live is in the store, and a FETCH gets it.

## The store

One SQLite file (`--store`), made where it is not there: the table `lines` (room, number, name, text, `sent_at`, at), keyed by room and number, with `PRAGMA user_version` 1. Nothing is taken out of it (Q42, decided on 2026-10-24: kept for good). A line is in the file, synced to the disk (`synchronous = FULL`), before it is published, so that a power loss does not take back a line that was seen and give its number to another.

**Each gatherer runs with a store of its own.** The file is held with `locking_mode = EXCLUSIVE`, and a second gatherer on it stops at the start (`the store ... is in use`). WHY NOT one file for gatherers of different rooms, which WAL allows on one machine: each would wait for the others' writes, and nothing would keep two from holding the same room. What the SQLite documentation says, and why SQLite and not JSON Lines, at the top of `src/bin/xmoq-chat/storage.rs`, the one file that uses `rusqlite`. The price of FULL is a sync of the disk for every line, which was not measured.

| Crate | Used for | License | Needs to build |
|---|---|---|---|
| `rusqlite` 0.40.2, `bundled` (`libsqlite3-sys` 0.38.2) | The store | MIT; SQLite is in the public domain | A C compiler |

## What is where

The library (`src/*.rs`) decides, and does no input or output; the tests are there. The names of the namespaces and tracks, the payloads of `said` and `lines`, and the limits of a line with their defaults are in [`xmoq-chat-tracks`](../chat-tracks/README.md), which the terminal viewer uses too without the store's C.

| Module | What it decides |
|---|---|
| `rate` | Whether a person is over the rate |
| `said_again` | Whether, and after how long, a participant is asked again for its `said` |
| `rooms` | The numbers of the lines of a room |
| `gather` | What becomes of a line: taken, or dropped and why; who is followed, and whether a room is left empty |
| `args` | What the flags mean, the limits, and which rooms are taken |

How long to wait before connecting again and before asking for a `SUBSCRIBE_NAMESPACE` again, what a FETCH gets, and the tasks that write each stream are [`xmoq-track-serving`](../track-serving/README.md)'s.

The binary (`src/bin/xmoq-chat/`): `session.rs` (the requests and the answers on one session), `storage.rs` (the store), `main.rs` (connecting again, and the signals). What it prints, one line for each thing it does (`WATCHING`, `PARTICIPANT`, `SUBSCRIBED`, `ROOM_PUBLISHED`, `LINE`, `LINE_DROPPED`, `FETCH_ANSWERED`, ...), is what `interop/chat.ts` waits for.

## Not done

- **Authorization for the demo alone** (Q51, provisional). With `--auth-key-hex <hex>`, the key the relay is given under the same flag, the gatherer makes tokens of the room's form itself (`xmoq-room-token`): `sub` in `xmoq/chat/*` for its SETUP, its SUBSCRIBE_NAMESPACE and its SUBSCRIBEs, `pub` in `xmoq/chat/*` for its PUBLISHes, each made anew for its request and good for 600 s, as the room's. Without the flag it sends no token. A gatherer of a real service would not hold the relay's key. Who may announce `xmoq/chat/<room>/<name>` is the relay's check of the participant's token.
- **The own relay keeps 256 requests of one session** (`MAX_PEER_REQUESTS` of `xmoq-moqt`): one gatherer follows about 250 participants through it. Ten rooms of a hundred take a gatherer a room (above). Not measured.
- **A `PUBLISH_OK` with Forward State 0** (§9.5: a relay "MAY respond with PUBLISH_OK in Forward State=0 until there are known subscribers") leaves the room not live for good: its lines wait (64 at the most) and are not sent, and a `REQUEST_UPDATE` with FORWARD 1 is refused as any update that carries more than delivery timeouts, which ends the PUBLISH. The own relay answers with Forward State 1; moqtail's 0.15.0 did in every run here. A FETCH still gets the lines.
- Lines said while the gatherer is away are lost: it does not FETCH what it missed when it subscribes again.
- A `NAMESPACE_DONE` that comes while a `SUBSCRIBE_NAMESPACE` is being asked for again is not seen: that participant stays followed, and its room published, until the gatherer's next session; it is asked again for its `said` `--ask-again-times` times.
- `sentAt` is passed on as the sender gave it; the gatherer does not compare it with its clock.

## Checks

`interop/chat.ts` (`mise run interop`, the pairings `<relay> relay: chat: ...`): two participants of a room, a live viewer and a late one, a line too long, lines too fast, and a FETCH cut to its limit, through the own relay and through `moqtail`'s; a participant that ends the gatherer's subscription with `PUBLISH_DONE` and is asked again; through the own relay also a restart on the same store with `--room r`, a participant of another room that is not taken, and the end of the room when everybody has left ([record](../../docs/experiments/2026-10-24-chat-gatherer.md); asking a participant again: [record](../../docs/experiments/2026-10-24-the-gatherer-asks-a-participant-again.md)).

`interop/chat-pages.ts` (the pairings `<relay> relay: chat: own pages ...`): own pages as participants with `joinChat` of [`@xmoq/chat`](../../packages/chat/README.md), which the demo's pages use, through both relays; a page that comes later gets the newest 16 lines by a Joining FETCH, which the own relay passes to the gatherer and moqtail's 0.15.0 answered from what it held, without asking the gatherer ([record](../../docs/experiments/2026-10-24-demo-chat-over-tracks.md)); and a page that stops withdraws its namespace, and the gatherer lets it go (`PARTICIPANT_GONE`) while the others go on ([record](../../docs/experiments/2026-10-24-a-page-withdraws-its-chat-namespace.md)).

`interop/chat-tokens.ts` (the pairing `own relay: chat: with tokens, ...`): through the own relay with `--auth-key-hex`, the gatherer with the same key and two pages with the room's two tokens: their lines reach each other through the gatherer, and a page that comes later has them by a Joining FETCH the gatherer answers ([record](../../docs/experiments/2026-10-25-the-chat-with-tokens-through-the-gatherer.md)).

`interop/watch-rust.ts --watch chat` (the pairings `<relay> relay: watch: two terminal viewers chat ...`): two terminal viewers of a room as participants, through both relays; the first comes before the gatherer has published the room's `lines`, and the later one gets the earlier line by its Joining FETCH.
