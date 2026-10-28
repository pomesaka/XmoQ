# xmoq-pet

The pet of a room. A client of the relay, beside it as the chat's gatherer ([`xmoq-chat`](../chat/README.md)) is, and an actor of every room's garden, named `pet`, as a person's page is one. A room has a garden, which the garden ([`xmoq-garden`](../garden/)) publishes: the things on its floor and what happened in it of late. Nobody acts on the pet: people act on the garden, the pet perceives the garden as a page does, decides what it does, and acts on the garden in its turn. The tracks and their payloads are [`xmoq-pet-tracks`](../pet-tracks/src/lib.rs)'s (the contract, `fixtures/pet-tracks/README.md`); a page's side is [`@xmoq/pet`](../../packages/pet/).

```sh
mise run pet -- --url https://127.0.0.1:4443/ --cert-sha256 <64 hex digits> --auth-key-hex <the relay's key>
DECISIONS_API_TOKEN=... mise run pet -- --url ... --cert-sha256 ... --decisions-url http://127.0.0.1:3333/v1/decisions
```

Beside a demo, give it the relay's port, the hash of `demo/.cert/cert.pem` and the key the demo gives the relay. It runs until Ctrl-C or SIGTERM, and connects again after a session ends and after a GOAWAY, by `NextSession` and `Pause::SERVING` of [`xmoq-again`](../again/src/lib.rs). With a flag it does not know it prints its flags, what each is without it and their ranges (`USAGE` in `src/args.rs`).

## What it asks for and publishes

| Request | Of what | Why |
|---|---|---|
| `SUBSCRIBE_NAMESPACE` | `xmoq/pet` | A room is learned of by the namespaces of the persons in it, `xmoq/pet/<room>/<name>`, as the garden learns of it; the garden announces no namespace of the room (`person_of` of `src/tracks.rs`) |
| SUBSCRIBE, Largest Object, and a Joining FETCH of the latest Group | `garden` of `xmoq/pet/<room>` | The garden as it is, and each change of it |
| `PUBLISH_NAMESPACE` | `xmoq/pet/<room>/pet` | The pet is an actor of the room |
| PUBLISH | `state` and `acted` of `xmoq/pet/<room>/pet` | Its body, and what it does to the garden |

It answers a FETCH of `state` with the latest state; the ones before it are an End of Unknown Range. A FETCH of `acted` is refused: what the pet did is taken live. With `--auth-key-hex` it makes its own tokens: `sub` for `xmoq/pet/*`, and `pub` for its namespace of the room alone.

## What it promises

**One pet a room**, which a room has from its first person until a while after its last one left (`FORGET_AFTER_MS` of `src/rooms.rs`), so that one who reloads the page finds the same pet. Then the pet withdraws its namespace and ends its tracks. It is held in memory alone: a pet that is forgotten, or whose program started again, begins anew.

**It perceives what is new.** Each garden read is compared with the one before (`src/perceive.rs`): the events after the last one seen, the things put on the floor since, and the things gone. The first garden of a room is taken as it is: what happened before the pet came is not reacted to.

**One decision a batch.** What is new of a room is gathered for `BATCH_MS` after the first of it (`src/batch.rs`), and the pet reacts once. The decisions API decides where `--decisions-url` is given (`src/decisions.rs`: what is asked, the pet, the things on the floor and what happened, and what is read of the answer); the table decides where it is not, where its token is not set, past `--decisions-per-minute`, and where the API gives no answer (`table` of `src/decide.rs`, by the newest happening).

**It looks at the floor when it goes back to itself.** What stays on the floor is no happening, so the pet also decides once, with nothing new, when it finished eating, when it took the ball, and when it would wander again after `approach`, `flee`, `jump` or `sit` upon a batch, where something it has not acted on lies on the floor (`look` of `src/pet.rs`, where the reason is). The decision is asked as any, with `happened` empty and the instructions saying that nothing new happened, and counted against `--decisions-per-minute`; the table eats a bowl, else chases the ball. After `ignore` or `sleep`, and after a decision upon the floor that is not `eat` or `chase`, it does not look until something new happens: each look again takes a thing off the floor, so it does not go round. `eat` is offered only with a bowl on the floor, and `chase` only with a ball. While the API is asked, the pet thinks: it stops where it is and sits with the bubble `?` until the decision comes; a batch that comes meanwhile waits for it. Where the API is given, one call with a made-up batch warms it up at the start, its answer thrown away (`warm_up` of `src/decisions.rs`, where the reason is).

**It acts on the garden where it arrives.** For `eat` it walks to the nearest bowl, sends `eat` of it on `acted` when it arrives, and eats for `EAT_MS` (`src/pet.rs`); for `chase` it runs to the ball and sends `take` of it. A thing gone by then is not acted on. `by` of its state is the actor of the happening it reacts to.

**A state a Group.** The `state` of a room is published while somebody is in it: at each change, and again without one every `STATE_AGAIN_MS`, so that one who comes gets a state soon.

**The API's token** is read once, from the environment variable `DECISIONS_API_TOKEN` alone, and goes into the request's `Authorization` header and nowhere else (`src/bin/xmoq-pet/decisions.rs`). Where `--decisions-url` is given without it, the pet says so on stderr and the table decides.

What it prints, one line for each thing it does: `DECISIONS_WARMED took_ms=..` or `DECISIONS_NOT_WARMED why=<word>`, `WATCHING`, `PERSON room=.. name=.. new_room=..`, `ANNOUNCED`, `SUBSCRIBED room=..`, `PUBLISHED room=.. track=..`, `PERCEIVED room=.. group=.. events=.. things=.. gone=.. floor=..` (for a garden with something new or gone), `GARDEN_DROPPED ... why=<word>`, `LOOKING room=.. floor=..` (a decision upon the floor begins), `THINKING room=.. happenings=.. floor=..` (`happenings=0` upon the floor), `DECIDED room=.. source=api|table why=<word> reaction=.. mood=.. bubble=..`, `STATE` (for each decision, not for each step of its wandering), `ACTED_SENT room=.. kind=eat|take thing=..` or `ACTED_NOT_SENT ... why=gone`, `FETCH_ANSWERED`, `PERSON_GONE`, `ROOM_EMPTY`, `ROOM_FORGOTTEN`.

| Crate | Used for | License | Needs to build |
|---|---|---|---|
| `reqwest` 0.13.5, `rustls-no-provider` | The decisions API | MIT or Apache-2.0 | Nothing more than `xmoq-viewer-io` |

## Not done

- **Authorization where the relay shares its key with the pet** (provisional), as the gatherer's: with `--auth-key-hex` it makes its own tokens.
- **The own relay keeps a limited number of requests of one session**: up to five of each room share them (`MAX_ROOMS` of `src/rooms.rs`). Not measured.
- The pet subscribes to `garden` when the first person of a room comes, which can be before the garden publishes it; it is asked again after the relay's refusal (`--ask-again-ms`, `--ask-again-times`), and not held for the publisher (`RENDEZVOUS_TIMEOUT`).
- One actor can fill a room's batch (`BATCH_MOST`); there is no limit for each actor.
- The places of people are not known: the pet comes to the front middle of the floor for a pet or a call.
- The ball the pet took is in its mouth for good: there is no action that drops it.
