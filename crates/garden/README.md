# xmoq-garden

The garden of a room. A client of the relay, beside it as the chat's gatherer ([`xmoq-chat`](../chat/README.md)) is: it is told of every actor of every room, takes what each does on its track `acted`, changes the room's garden by its rules (`src/rules.rs`), and publishes the garden as the track `garden` of the room. The tracks and their payloads are [`xmoq-pet-tracks`](../pet-tracks/src/lib.rs)'s, and the contract of the three sides (the garden, the pet, a page) is [`fixtures/pet-tracks/README.md`](../../fixtures/pet-tracks/README.md).

```sh
mise run garden -- --url https://127.0.0.1:4443/ --cert-sha256 <64 hex digits> --auth-key-hex <the relay's key>
```

Beside a demo, give it the relay's port and the hash of `demo/.cert/cert.pem` (`openssl x509 -in demo/.cert/cert.pem -outform der | openssl dgst -sha256`). The demo makes the relay's key anew each run and hands it only to the programs it starts, so a garden started by hand runs beside a demo started with `--no-auth`, without `--auth-key-hex`. It runs until Ctrl-C or SIGTERM, and connects again after a session ends and after a GOAWAY, by `NextSession` and `Pause::SERVING` of [`xmoq-again`](../again/src/lib.rs). With a flag it does not know it prints its flags, what each is without it and their ranges (`USAGE` in `src/args.rs`).

## What it promises

**One garden a room**, which a room has from its first actor until a while after its last one left (`FORGET_AFTER_MS` of `src/rooms.rs`), and is published that long. It is held in memory alone: a garden that is forgotten, or whose program started again, begins anew, its ids from 1.

**A garden a Group.** The `garden` of a room is published at each change, and again without one every `GARDEN_AGAIN_MS`, so that one who comes gets a garden soon. A FETCH of `garden` gets the latest garden; the ones before it are an End of Unknown Range.

**The garden says who may do what**: a person's `feed` and `throw` put a bowl or the ball down (one ball a room, a new id each throw), and `call`, `pet` and `talk` are events; the pet's `eat` and `take` take a bowl or the ball away. The rest is dropped and said in the log.

What it prints, one line for each thing it does: `WATCHING prefix=xmoq/pet`, `ACTOR room=.. name=..`, `SUBSCRIBED room=.. name=..`, `ROOM_PUBLISHED room=..`, `ACTED room=.. name=.. kind=..`, `ACTED_DROPPED room=.. name=.. why=<word>` (a word of `read_action`'s errors, `not-followed`, `not-the-pet`, `not-a-person` or `no-such-thing`), `GARDEN room=.. things=.. events=..` (for each change, not for each garden sent again), `FETCH_ANSWERED`, `ACTOR_GONE`, `ROOM_FORGOTTEN`.

## Not done

- **Authorization where the relay shares its key with the garden** (provisional), as the gatherer's: with `--auth-key-hex` it makes its own tokens for `xmoq/pet/*`.
- **The own relay keeps a limited number of requests of one session**: the PUBLISH of each room and the SUBSCRIBE of each actor share them, as the gatherer's do. Not measured.
- One actor can fill a room's garden with things and events; there is no limit for each actor.
