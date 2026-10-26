# The moqtail relay turns the moqtail Rust client away in HTTP/3, once

- Date: 2026-10-27
- Commit SHA: the CI run on `4f5eee65` (`main` after #523); the local runs on the change of #523 before its merge
- Roadmap item: none; a failure of the interop run between two peers of others

## Objective

One pairing failed in the CI run of `main` after #523 ([run 40377610814](https://github.com/pomesaka/XmoQ/actions/runs/40377610814), first attempt): "moqtail relay: own-browser only announces the namespace, and serves moqtail-rust when the relay asks it", with `sub:moqtail-rust exited with code 1 before printing /^SUBSCRIBED /` after 0.9 s. #523 had moved how a page answers a SUBSCRIBE (`publishTrack` of `@xmoq/tracks`), so it was to be found out whether the change was the cause, and whether the failure comes again.

## Environment

- CI: `ubuntu-24.04`, Chromium 156.0.8078.4, the moqtail relay `0.15.0+dd8623d` (tag `relay@0.15.0`), the moqtail Rust client of `interop/peers/moqtail/rust-client` (`moqtail` 0.15.0, `wtransport` 0.7.2).
- Local: macOS 14.4, arm64; Bun 1.3.14 of `mise.toml`; Chromium 156.0.8078.4, the same relay and client.
- MoQ Transport draft 18.

## Commands

The log of the pairing, from the artifact `interop-logs` of the run. Then, locally, eight times, with the lock of the interop runs:

```sh
mise run interop -- --base-port 46000 --only "only announces the namespace"
```

## Observed results

**In the CI run**, the page's part went as it is to: SETUP, PUBLISH_NAMESPACE of `xmoq/phase0`, its REQUEST_OK, and `namespace announced`. 17 ms after it, before the subscriber had sent a MoQ message, the relay ended the subscriber's connection (its session 1):

```text
[relay] ERROR relay::server: Error occurred in session 1: connection locally aborted: ExcessiveLoad
[sub:moqtail-rust] Error: WebTransport connect failed
[sub:moqtail-rust]     connection closed by peer: 263
```

263 is `0x107`, `H3_EXCESSIVE_LOAD` of HTTP/3 (RFC 9114 §8.1). `wtransport-proto` 0.7.2 (tag `0.7.2`, read on 2026-10-27) turns a frame of an HTTP/3 stream whose payload is longer than `MAX_PARSE_PAYLOAD_ALLOWED`, 4096 bytes (`frame.rs`), into `ParseError::PayloadTooBig`, and that into `ErrorCode::ExcessiveLoad` (`stream.rs`). So the relay read, on the connection of the moqtail Rust client, an HTTP/3 frame of more than 4096 bytes. Which frame it was, and why, the log does not show. The page's connection was not the one closed, and the page had not been asked for the track yet: `publishTrack` had nothing to answer.

None of the other 248 logs of the same artifact has `ExcessiveLoad` or a close with 263. The pairing passed in the CI run of #523 before its merge.

**Locally**, the eight runs passed, 6 of 6 pairings each, the failed pairing among them; no log had `ExcessiveLoad` or 263.

## Limitations and blockers

- The cause is a guess beyond what is above: an HTTP/3 frame that the moqtail Rust client sent, or that the relay read as one, longer than wtransport takes; both ends are wtransport 0.7.2. Not looked for in the client's source, and not reproduced.
- The runs are not changed to ask again where a peer of others fails to connect: that would hide a failure of the peers.

## Open questions raised

None for this repository. Where it comes again, the log of the relay is the one to read, for which stream the frame was on.
