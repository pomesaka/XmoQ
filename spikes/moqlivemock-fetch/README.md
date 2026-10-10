# Spike: FETCH bytes from moqlivemock

Outside lint and review. What was used to take the bytes in [the record](../../docs/experiments/2026-10-11-moqlivemock-fetch.md). Nothing here is built by the repository: the Go files belong into a copy of `Eyevinn/moqlivemock` at `v0.16.1`, kept outside the repository.

| File | What it is |
|---|---|
| `moqlivemock-v0.16.1.patch` | The changes to the copy: its clients and servers wrap their connection in the tee, and (from an earlier record) its clients send the `:protocol` the own relay takes |
| `tee.go` | `internal/tee/tee.go` of the copy: prints the bytes of every stream as the library reads and writes them, when `MLM_TEE` is set |
| `fetchcap.go` | `cmd/fetchcap/main.go` of the copy: a publisher that answers a FETCH with scripted records through `Eyevinn/moqtransport`, and a client that sends a Standalone FETCH. Every byte is encoded by that library |
| `decode-rs.rs`, `decode-ts.ts` | The two programs that read the captured bytes with the own Rust and TypeScript decoders, a byte at a time. The Rust one is a crate of its own with a path dependency on `crates/moqt` |
| `out/scripted-streams.txt` | The five fetch streams the scripted publisher wrote: name, Group Order, bytes |
| `out/messages.txt` | The control messages that were captured |

The two streams of video taken from `mlmrel` are not kept: they are 92,881 bytes each of moqlivemock's test asset.
