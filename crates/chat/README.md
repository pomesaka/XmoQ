# xmoq-chat

The gatherer of the chat. A client of the relay that is told of every participant of every room, takes the lines each one says, numbers the lines of a room in the order they reach it, keeps them in a SQLite file, and publishes them as one track a room. A viewer gets the lines live by SUBSCRIBE and the older ones by FETCH, which the relay passes to the gatherer. The tracks and their payloads are [`xmoq-chat-tracks`](../chat-tracks/README.md)'s.

```sh
mise run chat -- --url https://127.0.0.1:4443/ --cert-sha256 <64 hex digits> --store chat.sqlite3
mise run chat -- --url ... --cert-sha256 ... --store room-r.sqlite3 --room r     # one room alone
```

It runs until Ctrl-C or SIGTERM, and connects again after a session ends and after a GOAWAY, by `NextSession` and `Pause::SERVING` of [`xmoq-again`](../again/src/lib.rs). With a flag it does not know it prints its flags, what each is without it and their ranges (`USAGE` in `src/args.rs`). `--room <room>` has it take, publish and answer FETCH for those rooms alone.

**Building it needs a C compiler**: `rusqlite` builds SQLite from the source it carries (`bundled`), through the `cc` crate. On macOS the Command Line Tools' `clang`, on Linux `gcc`.

## What it promises

**Order**: the lines of a room are numbered from 0, without a gap, in the order they reach the gatherer; `sentAt` plays no part in it.

**A line is kept for good**: nothing is taken out of the store, and a line is in the file, synced to the disk, before it is published. A line that is not sent live is in the store, and a FETCH gets it. Why a line is not taken (`LINE_DROPPED ... why=<word>`) is `Dropped` of `src/gather.rs`, and why a reaction is not counted (`REACTION_DROPPED`) `ReactionDropped` of `src/react.rs`.

**The counts are not kept**: nothing of `reactions` and `present` is in the store, a FETCH for either is refused with `NOT_SUPPORTED`, and the next Object says the state anew.

**Each gatherer runs with a store of its own.** A second gatherer on the same file stops at the start (`the store ... is in use`). Why, and why SQLite, at the top of `src/bin/xmoq-chat/storage.rs`, the one file that uses `rusqlite`.

| Crate | Used for | License | Needs to build |
|---|---|---|---|
| `rusqlite` 0.40.2, `bundled` (`libsqlite3-sys` 0.38.2) | The store | MIT; SQLite is in the public domain | A C compiler |

What it prints, one line for each thing it does (`WATCHING`, `PARTICIPANT`, `SUBSCRIBED`, `ROOM_PUBLISHED`, `LINE`, `LINE_DROPPED`, `PRESENT`, `REACTIONS`, `REACTION_DROPPED`, `FETCH_ANSWERED`, ...), is what `interop/chat.ts` waits for.

## Not done

- **Authorization where the relay shares its key with the gatherer** (provisional). With `--auth-key-hex <hex>`, the key the relay is given under the same flag, the gatherer makes tokens of the room's form itself (`xmoq-room-token`): `sub` in `xmoq/chat/*` for its SETUP, its SUBSCRIBE_NAMESPACE and its SUBSCRIBEs, `pub` in `xmoq/chat/*` for its PUBLISHes, each made anew for its request and good for 600 s, as the room's. Without the flag it sends no token. A gatherer of a real service would not hold the relay's key. Who may announce `xmoq/chat/<room>/<name>` is the relay's check of the participant's token.
- **The own relay keeps a limited number of requests of one session** (`MAX_PEER_REQUESTS` of `xmoq-moqt`, or the relay's `--peer-requests-per-session`): one gatherer follows that many participants through it, less the `SUBSCRIBE_NAMESPACE` and the PUBLISH. Ten rooms of a hundred take a gatherer a room. Not measured.
- **A `PUBLISH_OK` with Forward State 0** (§9.5: a relay "MAY respond with PUBLISH_OK in Forward State=0 until there are known subscribers") leaves the room not live for good: its lines wait (64 at the most) and are not sent, and a `REQUEST_UPDATE` with FORWARD 1 is refused as any update that carries more than delivery timeouts, which ends the PUBLISH. The own relay answers with Forward State 1; moqtail's 0.15.0 did in every run here. A FETCH still gets the lines.
- Lines said while the gatherer is away are lost: it does not FETCH what it missed when it subscribes again.
- A `NAMESPACE_DONE` that comes while a `SUBSCRIBE_NAMESPACE` is being asked for again is not seen: that participant stays followed, and its room published, until the gatherer's next session; it is asked again for its `said` `--ask-again-times` times.
- `sentAt` is passed on as the sender gave it; the gatherer does not compare it with its clock.
- The price of `synchronous = FULL`, a sync of the disk for every line, was not measured.
