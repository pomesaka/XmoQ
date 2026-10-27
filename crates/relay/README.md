# xmoq-relay

The relay of XmoQ: MoQ Transport draft 18 over WebTransport, on one machine. What it does and what is decided where is at the top of `src/main.rs`; who gets what and who is cut off is [`xmoq-relay-core`](../relay-core/src/lib.rs), and the streams are [`xmoq-moqt-io`](../moqt-io/src/lib.rs). Where draft 18 is silent or contradicts itself, what it does is in the relay's table of the [implementation notes](../../docs/moqt-implementation-notes.md).

## Run

`cargo run -p xmoq-relay -- <flags>`. `demo/serve.ts` and the interop run start it with their own flags.

## Flags

The flags and what each is without it are `USAGE` in `src/main.rs`, which the relay prints when a flag is wrong. The limits are there to be fitted to the machine it runs on: at worst a session holds about 23.4 MiB besides what waits to be written to it, so the relay holds `max-sessions × 23.4 MiB + min(max-sessions × session-send-bytes, relay-send-bytes)`, about 25 GiB with the defaults (the table of `HubConfig` in `src/hub.rs` has each part). What one edge with 500 viewers took is in [a record](../../docs/experiments/2026-10-25-one-video-track-through-two-edges.md).
