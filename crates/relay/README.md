# xmoq-relay

The relay of XmoQ: MoQ Transport draft 18 over WebTransport, on one machine. What it does and what is decided where is at the top of `src/main.rs`; who gets what and who is cut off is [`xmoq-relay-core`](../relay-core/src/lib.rs), and the streams are [`xmoq-moqt-io`](../moqt-io/src/lib.rs). Where draft 18 is silent or contradicts itself, what it does is in the relay's table of the [implementation notes](../../docs/moqt-implementation-notes.md).

## Run

`cargo run -p xmoq-relay -- <flags>`. `demo/serve.ts` and the interop run start it with their own flags.

## Flags

Read off `src/main.rs` on 2026-10-25. The limits are there to be fitted to the machine it runs on: at worst a session holds about 23.4 MiB besides what waits to be written to it, so the relay holds `max-sessions × 23.4 MiB + min(max-sessions × session-send-bytes, relay-send-bytes)`, about 25 GiB with the defaults (the table of `HubConfig` in `src/hub.rs` has each part); one edge with 500 viewers took 192 MB.


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
| `--upstream`, `--upstream-cert-sha256`, `--upstream-namespace` | The relay above this one, asked for the tracks under that namespace that nobody announced here; the first two go together | No upstream; every namespace with `--upstream` alone |
| `--redirect-uri` | The New Session URI of the GOAWAY the relay sends when it is told to stop | Empty: the URI the client used |
| `--goaway-timeout-ms` | The Timeout of that GOAWAY, and how long the relay waits before it closes the sessions that are left | 10,000 |
| `--stats-every-ms`, `--log-arrivals on` | For measuring: a `STATS` line for each session at that interval, and an `ARRIVED` line on stderr for every Object that arrives | Off |
| `--slow-open-ms` | For a run that looks at how long streams take to open; 0 logs every stream | Not logged |
