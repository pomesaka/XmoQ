# What a writer fails with in Chromium

Spike, outside lint and review. It is the measurement of [the record](../../docs/experiments/2026-10-11-write-abort-in-chromium.md): what `writer.closed` and `write()` of a WebTransport stream are rejected with when the peer stops the stream, when the session ends, and when the connection is lost.

| File | Content |
|---|---|
| `src/main.rs` | A WebTransport server on `web-transport-quinn` 0.13.2 that stops streams, closes the session, or holds everything |
| `run.ts` | Starts the server and Chromium, runs the samples on raw `WebTransport`, and prints how often each outcome was seen |
| `out/rounds-60.txt` | What the run of the record printed |
| `out/peer-opened-40.txt` | `run.ts --opened-by peer`: the same on a bidirectional stream the server opened, with a server that also resets its direction (the `/peer-` paths) |
| `out/peer-opened-stop-now-40.txt`, `out/own-opened-stop-now-40.txt` | `--only stop-now`, with and without `--opened-by peer`: a STOP_SENDING at once, with how long the writer took to hear of it and how the server saw each session end |
| `own-reset.ts` | The same server and browser: what a writer and a write under way fail with when the page aborts the writer itself |
| `out/own-reset-20.txt` | What that printed |

```sh
(cd spikes/wt-write-abort && cargo build --locked)
bun spikes/wt-write-abort/run.ts --rounds 60 --out spikes/wt-write-abort/out/rounds-60.txt
```

It needs the certificate an interop run leaves in `interop/.cert/`, and the Chromium of `interop/`.
