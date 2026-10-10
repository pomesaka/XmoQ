# WebTransport error code probe

Throwaway code that answers one question: what does a browser see when a `wtransport` 0.7.2 server closes a session, resets a stream or sends STOP_SENDING, each with an error code? It is not part of the application. Results are in [the own relay record](../../docs/experiments/2026-10-10-own-relay-vs-moqtail-peers.md).

It uses the certificate and the Playwright install of [`spikes/moqtail`](../moqtail/README.md); follow the steps there first.

```sh
cargo build --manifest-path spikes/wt-error-codes/Cargo.toml
node spikes/wt-error-codes/probe.mjs          # codes passed to wtransport as they are
node spikes/wt-error-codes/probe.mjs --map    # stream codes remapped into the HTTP/3 range first
```

| Path | Content |
|---|---|
| `src/main.rs` | The server. The request path picks what it does: `/close`, `/reset`, `/stop`, `/observe`, `/protocol` |
| `probe.mjs` | Starts the server and a headless Chromium, and prints what the page and the server saw |
