# xmoq-moqt-client

A MoQ Transport draft 18 client over WebTransport, on [`xmoq-moqt`](../moqt/README.md) and [`xmoq-moqt-io`](../moqt-io/src/lib.rs): with `--mode` it subscribes to a track, publishes one, announces a namespace, subscribes to one, sends FETCH requests or a TRACK_STATUS, and prints what happens. It is the own Rust peer of the [interop runs](../../interop/README.md), which build it (`target/debug/xmoq-moqt-client`) and drive it; nobody watches a broadcast with it (the terminal viewer is [`xmoq-watch`](../watch/README.md)).

Its flags are the doc comments of `Args` and its parts in `src/main.rs`. Every line it prints to stdout is in `src/report.rs`, and the drivers read them, so their format is a contract with `interop/`. How a publisher answers a Standalone FETCH for its track with `--answer-fetch`, and where that differs from `@xmoq/moqt`, is the row "A publisher that answers a Standalone FETCH" of the [implementation notes](../../docs/moqt-implementation-notes.md).
