# Spike: the demo over a link that loses and delays packets

A UDP relay between the watching page and the MoQ relay that drops, delays and jitters datagrams, and a script that runs the demo through it. No setting of the system is touched. Record: [docs/experiments/2026-10-11-demo-over-a-lossy-link.md](../../docs/experiments/2026-10-11-demo-over-a-lossy-link.md).

```sh
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384     # in the repository root
node spikes/lossy-link/proxy.mjs --listen 45386 --to 45382 --control 45387
bun spikes/lossy-link/measure.ts http://localhost:45381 45386 http://127.0.0.1:45387 "$PWD/interop"
```

`reconnect.ts` (a cut of 8 seconds, for the viewer and for the publisher) and `audio-under-loss.ts` (the sound with a longer buffer and with several chunks in a Group) take the same four arguments.

By hand: open `/publish`, then `/watch?via=45386`, and change the link with `curl 'http://127.0.0.1:45387/set?loss=0.05&delay=50&jitter=10'`.
