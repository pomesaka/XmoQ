<div align="center">
  <img src="docs/assets/xmoq-logo.svg" alt="XmoQ logo" width="160">
  <h1>XmoQ</h1>
  <p><strong>Experimental real-time media over QUIC.</strong></p>
</div>

> [!IMPORTANT]
> **Early implementation.** No working streaming implementation is claimed. No media has been sent. What exists is an own MoQ Transport implementation: a client in Rust and in TypeScript and a minimal relay in Rust, which exchange single Objects with each other and with `moqtail` peers.

XmoQ explores one-to-many live streaming using MoQ/WebTransport, browser WebCodecs, a Rust relay and terminal CLI, and Cloudflare Durable Objects for application state. Its purpose is to let an agent such as Claude take part in a broadcast, and later in a meeting of up to 49 people: a CLI subscribes and writes what the tracks carry where the agent reads it ([vision](docs/vision.md)). Integration with events in a separately developed SNS is a future direction, not committed.

## Architecture at a glance

![Proposed XmoQ architecture](docs/assets/architecture.svg)

*Conceptual design — interoperability and integration are not yet validated.*

## Documentation

- [Vision](docs/vision.md)
- [Requirements](docs/requirements.md)
- [Architecture and broadcast flow](docs/architecture.md)
- [Scaling a broadcast and meetings: a design draft](docs/scale-and-meetings.md)
- [Protocol layers](docs/protocols.md)
- [MoQ Transport implementation notes](docs/moqt-implementation-notes.md)
- [Wire fixtures captured from moqtail](fixtures/moqt-18/README.md)
- [Bytes written by hand from the draft, read by both implementations](fixtures/moqt-18-by-hand/README.md)
- [The points at which a stream of the peer has said what it is](fixtures/first-bytes/README.md)
- [What becomes of a SUBSCRIBE the peer sends](fixtures/peer-subscribe/README.md)
- [What becomes of a FETCH the peer sends](fixtures/peer-fetch/README.md)
- [Whether the SUBSCRIBE of a Joining FETCH is yet to come](fixtures/joining-request-id/README.md)
- [What a publisher answers a FETCH range with, from what it published](fixtures/published-fetch/README.md)
- [Catalogs of MSF, from another implementation and written by hand](fixtures/msf-01/README.md)
- [When picture and sound are played: what the page and the terminal viewer agree on](fixtures/playout/README.md)
- [How long a request waits for its answer](fixtures/request-answer-wait/README.md)
- [The Retry Interval of a REQUEST_ERROR](fixtures/retry-interval/README.md)
- [What a receiver makes of Immutable Properties](fixtures/immutable-properties/README.md)
- [A LOC video stream of another implementation](fixtures/loc-04/README.md)
- [Browser client library `@xmoq/moqt`](packages/moqt/README.md)
- [The Low Overhead Media Container in TypeScript, `@xmoq/loc`](packages/loc/README.md)
- [The MOQT Streaming Format in TypeScript, `@xmoq/msf`](packages/msf/README.md)
- [Between a video track and a decoder, `@xmoq/media`](packages/media/README.md)
- [A participant of the chat over MoQ tracks, `@xmoq/chat`](packages/chat/README.md)
- [The reader of JSON that the packages of the tracks share, `@xmoq/json`](packages/json/README.md)
- [The viewer for the terminal, `xmoq-watch`](crates/watch/README.md)
- [Pictures of a video track as PNG, `xmoq-picture`](crates/picture/README.md)
- [The gatherer of the chat, `xmoq-chat`](crates/chat/README.md)
- [The reader of a room for an agent, `xmoq-agent`](crates/agent/README.md)
- [The tracks of the chat, `xmoq-chat-tracks`](crates/chat-tracks/README.md)
- [The payloads of the chat's tracks, shared by the implementations](fixtures/chat-tracks/README.md)
- [When a track is asked for again on the same session, `xmoq-again`](crates/again/README.md)
- [Serving tracks of one Object a Group and answering FETCH, `xmoq-track-serving`](crates/track-serving/README.md)
- [The transcriber, `xmoq-transcriber`](crates/transcriber/README.md)
- [Cutting sound and reading it with whisper.cpp, `xmoq-speech`](crates/speech/README.md)
- [The tracks of the transcripts, `xmoq-transcript-tracks`](crates/transcript-tracks/README.md)
- [A reader of the transcripts over MoQ tracks, `@xmoq/transcript`](packages/transcript/README.md)
- [A participant of a meeting in a page, `@xmoq/meeting`](packages/meeting/README.md)
- [The payload of a transcript, shared by the implementations](fixtures/transcript-tracks/README.md)
- [A voice for the transcriber's interop run](fixtures/speech/README.md)
- [Interop runs against moqtail peers](interop/README.md)
- [MoQ library candidates](docs/moq-libraries.md)
- [The audio and video demo: what it does and where it ends](docs/av-demo.md)
- [Roadmap](docs/roadmap.md)
- [Validation](docs/validation.md)
- [Open questions](docs/open-questions.md)
- [Experiment records](docs/experiments/README.md)
- [Agent guide](docs/agent-guide.md)

## Checks

Tool versions are pinned in `mise.toml` and `rust-toolchain.toml`. Run `mise install` once. The Rust build also needs a C compiler, which mise does not install: `xmoq-chat` builds SQLite from source (on macOS the Command Line Tools' `clang`, on Linux `gcc`), and `xmoq-watch` builds OpenH264 with a C++ compiler. Then:

| Command | What it runs |
|---|---|
| `mise run check` | Everything CI runs |
| `mise run check:rust` | `cargo fmt --check`, `cargo clippy` with warnings as errors, `cargo test` |
| `mise run check:ts` | Biome (format and lint), oxlint (`as` assertions), `typecheck` for packages and `interop/`, `bun test` per package |
| `mise run check:layers` | Dependency rules: `xmoq-moqt` stays free of I/O crates, nothing depends on `spikes/`, packages do not depend on `interop/` |
| `mise run check:workflows` | `actionlint` and `shellcheck` |
| `mise run check:docs` | The index of `docs/experiments/` is what `scripts/ci/experiments-index.sh` writes (`mise run fmt` writes it) |
| `mise run demo` | Starts the own relay and a web server; a page publishes video (a painted clock or the camera) and other pages watch it, in Chromium, with captions from the transcriber where its models are fetched. See [`demo/README.md`](demo/README.md). Outside lint and type checking |
| `mise run watch -- <flags>` | The viewer for the terminal, `xmoq-watch`: the picture of a broadcast in Ghostty or kitty, its sound, and the chat of its room. `-- --room http://127.0.0.1:8787/rooms/demo` next to `mise run demo`. See [`crates/watch/README.md`](crates/watch/README.md) |
| `mise run chat -- <flags>` | The gatherer of the chat, `xmoq-chat`: numbers the lines of every room, keeps them in a SQLite file and publishes them as one track a room. See [`crates/chat/README.md`](crates/chat/README.md) |
| `mise run agent -- <flags>` | The reader of a room for an agent, `xmoq-agent`: what happens in a room (its chat, who comes, pictures asked for, the words of a transcript) as lines of JSON on stdout and in a file; a line of stdin is said in the chat. See [`crates/agent/README.md`](crates/agent/README.md) |
| `mise run agent-demo -- start --out <dir>` | A demo of its own on ports apart from `mise run demo`'s, the reader in its room, and the publishing page in a headless Chromium with a voice as its microphone, all in the background; with `--meeting N`, N participants of the meeting (`/meet`) and the reader in it; `stop --out <dir>` ends what `<dir>/pids` names. See the head of [`scripts/agent-demo.sh`](scripts/agent-demo.sh) |
| `mise run transcriber -- <flags>` | The transcriber, `xmoq-transcriber`: reads the voices of the broadcasts under a prefix with whisper.cpp and publishes their words as one track a prefix. See [`crates/transcriber/README.md`](crates/transcriber/README.md) |
| `mise run speech-model [-- <model>]` | Fetches a model of whisper.cpp into `~/.cache/xmoq/whisper/` and checks its sha256. See [`crates/speech/README.md`](crates/speech/README.md) |
| `mise run interop` | Builds the `moqtail` relay and the Rust clients, then moves an Object through each relay for every pairing of own and `moqtail` clients, and runs the scenarios in which a client leaves through the own relay, in a real Chromium. Not part of `check` |
| `mise run fmt` | Apply formatting and safe fixes |

The relay (`cargo run -p xmoq-relay -- <flags>`; `demo/serve.ts` and the interop run start it) has these flags, read off `crates/relay/src/main.rs` on 2026-10-25. The limits are there to be fitted to the machine it runs on: at worst a session holds about 23.4 MiB besides what waits to be written to it, so the relay holds `max-sessions × 23.4 MiB + min(max-sessions × session-send-bytes, relay-send-bytes)`, about 25 GiB with the defaults (the table of `HubConfig` in `crates/relay/src/hub.rs` has each part); one edge with 500 viewers took 192 MB. The pages' and `demo/serve.ts`'s are in [the demo's README](demo/README.md), the terminal viewer's in `USAGE` of `crates/watch/src/args.rs`.

| Flag | What it sets | Without it |
|---|---|---|
| `--port`, `--cert-file`, `--key-file` | Where it listens, and its certificate | Required |
| `--congestion cubic\|newreno\|bbr` | The congestion control of its connections | cubic |
| `--idle-timeout-ms` | How long nothing may arrive from a peer before its connection counts as lost | 6,000 |
| `--setup-timeout-ms` | How long a client has for its SETUP | 10,000 |
| `--idle-session-ms` | How long a session that has sent its SETUP may hold nothing | 30,000 |
| `--max-sessions` | How many sessions it takes at once; more than 0 | 1,000 |
| `--subscription-send-bytes`, `--subscription-send-objects`, `--subscription-send-streams` | What may wait to be written to one subscription before it ends (`TOO_FAR_BEHIND`); more than 0 | 4 MiB, 4,096, 1,024 |
| `--session-send-bytes`, `--session-send-objects`, `--session-send-streams` | What may wait to be written to one session, all its subscriptions together, before the one that carries the most ends; more than 0 | 8 MiB, 16,384, 4,096 |
| `--relay-send-bytes` | What may wait to be written to all the sessions together, before the subscription with the most waiting ends; more than 0 | 2 GiB |
| `--when-full` | Where a session goes that comes when it is full: a GOAWAY after its SETUP names this URL with `sent-away=1` in its query, and a session that came with that is closed rather than sent on. The relay there has to have the same certificate, or one the clients take otherwise | Closed with `INTERNAL_ERROR` |
| `--announce-at-once` | How many `PUBLISH_NAMESPACE` requests a session may send at once | 5 |
| `--announce-every-ms` | The time in which a session gets one of them back | 1,000 |
| `--peer-requests-per-session`, `--own-requests-per-session`, `--namespaces-per-session` | How many requests a session may have open at the relay, how many the relay may have open on it, and how many namespaces it may have announced; for every session alike, so a relay above edges that carry a meeting is given more, and its viewers have as much ([design draft](docs/scale-and-meetings.md)); more than 0 | 256, 64, 64 |
| `--requests-for-one-session` | How many requests the relay has open on one publisher for one subscriber's session | 8 |
| `--fetches-for-one-session` | How many of them are FETCH requests | 4 |
| `--cached-fetch-whole-ms` | How long a subscriber has for the whole of a fetch stream the relay writes from its cache, at the least | 30,000 |
| `--held-beyond-cache-bytes` | How many bytes the fetch streams it writes may keep alive of Groups its cache has let go of; a session has an eighth | 268,435,456 |
| `--cached-fetch-least-kbps` | The rate it has to take such a stream at, at the least: a larger Group has as long as it takes at that rate | 500 |
| `--delivery-patience-ms` | Experiment: what a subscriber gets that asked for no delivery timeout | Off |
| `--auth-key-hex`, `--auth-setup off` | Experiment: the key tokens are checked with, and whether SETUP is asked for one | No tokens; on with a key |
| `--slow-open-ms` | For a run that looks at how long streams take to open; 0 logs every stream | Not logged |

CI (`.github/workflows/ci.yml`) runs the same tasks. A task whose target does not exist yet prints `SKIP` and succeeds; `spikes/` is outside every check, and so are the copies of `moqtail` code under `interop/peers/`. The interop run is a CI job of its own, which calls `mise run interop`; for a pull request that touches nothing the run depends on, the job reports that it skipped the pairings.

License, deployment, supported platforms, codec profiles, authentication, and performance targets are undecided.
