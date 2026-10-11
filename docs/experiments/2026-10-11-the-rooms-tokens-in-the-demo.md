# Tokens of the room, carried by the pages of the demo to the relay

- Date: 2026-10-11
- Commit SHA: the commit that adds the option and the room's tokens, together with this record. The relay that checks tokens was not in this tree yet: it was built from the change that adds it (`npkwpwkr`), copied out of the repository, and the demo was run with that binary
- Roadmap item: none closed. The browser's side of the experiment in authorization; **how the relay authorizes is undecided**

## Objective

Have the room give out the tokens the relay's experiment checks, and the pages of the demo carry them in their requests: with a token, picture and sound; without one, a refusal that the page says.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay | `xmoq-relay` of `npkwpwkr`, with `--auth-key-hex`; and without it |
| Rooms | `spikes/durable-objects-room` on Miniflare 4.20260730.0, with `AUTH_KEY_HEX` |

## Commands

```sh
bun demo/serve.ts --auth --relay-binary <the relay of npkwpwkr> --port 45381 --relay-port 45382 --rooms-port 45384
(cd spikes/durable-objects-room && node check.mjs http://127.0.0.1:45384)
# a script outside the repository opened /publish and /watch, each with and without ?token=no
mise run check
mise run interop -- --base-port 45300
```

## What was built

- The room signs `v1|<namespace>|<pub or sub>|<expiry>|<mac>` with HMAC-SHA-256 (WebCrypto) and the key it shares with the relay, good for 600 seconds, for whoever asks on its WebSocket (`token-for`).
- `@xmoq/moqt` carried AUTHORIZATION TOKEN in every request already (`parameters.authorizationToken`, the Token structure as bytes). `playVideoTrack`, `playAudioTrack` and `followCatalog` got an option that puts one into their SUBSCRIBE and Joining FETCH.
- The pages ask the room for a token before each session: the publishing page a `pub` one for its PUBLISH_NAMESPACE, the watching page a `sub` one. The page wraps the text as a Token with Alias Type `USE_VALUE` (0x3) and Token Type 0.
- `bun demo/serve.ts --auth` makes a key for the run and gives it to the relay and the rooms.

## Observed results

| The relay asks for tokens | The page | Seen |
|---|---|---|
| yes | publishes, with a token | "publishing (session 1)" |
| yes | publishes, `?token=no` | "could not publish (RequestFailedError: REQUEST_ERROR 0x1: ); trying again every 2 s" |
| yes | watches, with a token | 120 frames and 200 pieces of audio in 4 seconds |
| yes | watches, `?token=no` | "could not play (RequestFailedError: REQUEST_ERROR 0x1: ); trying again every 2 s", nothing decoded |
| no | all four | publish and play as before, token or not |

- `0x1` is UNAUTHORIZED. The pages now say "the relay refused: not authorized (no token of the room, or one it does not take)" for it.
- **Run again with the relay of this tree**, once it had the change (`bun demo/serve.ts --auth`, without `--relay-binary`): the same four results, with that wording for the two refusals.
- A token of the room has the form the relay takes: `v1|xmoq/demo|sub|1791721868|<mac>` (`check.mjs`).
- The Joining FETCH of the watching page carries the token as well; the relay does not check a Joining FETCH (its record), so that says nothing.

## Limitations and blockers

- **The room gives a token to anybody who asks**, for any namespace and either role. It shows that the two ends fit, and decides nothing about who may publish or watch.
- A token is good for 600 seconds; a page asks for a new one for each session, and a session that is under way goes on after its token has expired (the relay's side).
- Not run: a token for another namespace, one that has expired, one with a wrong mac, from a page (the relay's own tests have those); the interop matrix has no run with tokens.
- One run each, headless.
