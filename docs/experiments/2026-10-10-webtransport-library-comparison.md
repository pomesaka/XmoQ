# WebTransport library comparison: wtransport and web-transport-quinn

- Date: 2026-10-10
- Commit SHA: not committed. The runs were made on a working copy on top of `b7a5cc4`
- Roadmap item: Phase 0b, own Rust relay against the `moqtail` browser client and the own Rust client (the WebTransport library under it)

## Objective

Collect facts for the choice of the WebTransport library under XmoQ's Rust side. `wtransport` 0.7.2 is in use. A probe in the `wt-error-codes` spike (session `iori-a95c`, not on `main`) showed that a browser does not receive the close code of a session a `wtransport` server closes, and receives stream error codes only if the caller maps them by hand. The source of `web-transport-quinn` 0.13.2 says it does both; this run checks that by running it, with the same probes on both libraries. This record decides nothing.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Libraries under test | `wtransport` 0.7.2 (feature `quinn`); `web-transport-quinn` 0.13.2 with `web-transport-proto` 0.6.2, `web-transport-trait` 0.5.0, `kio` 0.5.9 (feature `ring` instead of the default `aws-lc-rs`) |
| Shared below both | `quinn` 0.11.12, `quinn-proto` 0.11.19, `rustls` 0.23.45, `ring` 0.17.14, `tokio` 1.49.0 (`spikes/wt-library-comparison/Cargo.lock`) |
| Browser | Chromium 156.0.8078.4, new headless, through `playwright` 1.64.0 |
| Peers | moqtail `relay` 0.15.0 (`moqtail/0.15.0+dd8623d`); the spike client on the `moqtail` crate 0.15.0 and `wtransport` 0.7.2; `xmoq-moqt-client` 0.0.0 at `b7a5cc4`; `moqtail` npm 0.13.0 |
| Toolchain | `cargo` 1.99.0, `node` 23.10.0, LibreSSL 3.3.6 |
| Wire | WebTransport subprotocol `moqt-18`; MOQT draft 18 for the SETUP exchange |
| Certificate | self-signed ECDSA P-256, valid 13 days, made with `openssl ecparam -genkey` and `openssl req -x509 -new -key`; given to Chromium as `serverCertificateHashes` and to Rust clients as a pinned SHA-256 |

Versions were read from crates.io and the lockfile on 2026-10-10, not from memory. Library behaviour that is cited from source refers to the crate sources of these versions.

Both probe servers ran with `max_concurrent_uni_streams` 8, a stream receive window of 64 KiB, a connection receive window of 128 KiB and a 3 s keep-alive, except in the runs marked "default transport". None of these values is a project decision.

## Commands

From the repository root, after the build steps of [`spikes/moqtail/README.md`](../../spikes/moqtail/README.md). The code and the meaning of each probe are in [`spikes/wt-library-comparison`](../../spikes/wt-library-comparison/README.md).

```sh
cargo build --manifest-path spikes/wt-library-comparison/Cargo.toml
cargo build --workspace --locked

node spikes/wt-library-comparison/probe.mjs --library wtq --rounds 20
node spikes/wt-library-comparison/probe.mjs --library wt --rounds 20
node spikes/wt-library-comparison/probe.mjs --library wt-map --rounds 20
node spikes/wt-library-comparison/probe.mjs --library wtq --rounds 100
node spikes/wt-library-comparison/probe.mjs --library wt-map --rounds 100
node spikes/wt-library-comparison/probe.mjs --library wt-map-default --rounds 100   # twice
node spikes/wt-library-comparison/probe.mjs --library wtq-default --rounds 100
node spikes/wt-library-comparison/interop.mjs
```

A different `--port` was passed per run. Before these, one run of `--library wtq --rounds 3` checked the driver; its outcomes were the same as in the later `wtq` runs and are not counted below.

## Observed results

### What a browser and the server see of each other's codes

`wtransport`, mapped by hand: the probe server converts stream codes with `0x52e4a40fa8db + n + floor(n / 0x1e)` before it calls the library. Session codes cannot be mapped that way, because the code has to travel in a capsule the library does not send.

| Probe | `wtransport` 0.7.2 | `wtransport` 0.7.2, stream codes mapped by hand | `web-transport-quinn` 0.13.2 |
|---|---|---|---|
| Server closes the session with code 3 and a reason. Page reads `WebTransport.closed` | rejected: `WebTransportError`, `Connection lost.`, source `session`. No code, no reason | same | resolved: `closeCode: 3`, `reason: "protocol violation"` |
| Server resets a unidirectional stream with 0x12. Page reads `streamErrorCode` | 0 | 18 | 18 |
| Server sends STOP_SENDING with 0x5. Page reads `streamErrorCode` | 0 in 20 of 20 | 5 in 120 of 120; with default transport 5 in 198 of 200 | 5 in 120 of 120; with default transport 5 in 99 of 100 |
| Page aborts its stream with code 0x12. Server reads the stream | `Reset(91141958510829)`, the mapped value as sent | same | `Reset(18)` |
| Page closes with `closeCode: 3`, `reason: "bye"`. Server reads the session | `accept_uni` fails with `ApplicationClosed { code: 3, reason: "bye" }`; `closed()` returns `LocallyClosed` | same | `accept_uni` and `closed()` both return `WebTransportError(Closed(3, "bye"))` |

Three points the table does not show:

- **STOP_SENDING without a code happened on both libraries.** In the default-transport runs, 2 of 200 rounds on `wtransport` and 1 of 100 on `web-transport-quinn` rejected the page's write with `NetworkError: The stream was aborted by the remote server` and no `streamErrorCode`. With the limits of this spike it happened in none of 120 rounds on either. The `wt-error-codes` probe saw 2 of 5 without a code on `wtransport`; that rate was not reproduced. The cause was not looked for.
- **A stream the page resets right after its first write sometimes never reaches the application, on both libraries.** `accept_uni` did not return that stream in 3 of 20, 2 of 20, 10 of 100, 8 of 100 and 11 of 100 rounds on `wtransport`, and in 1 of 20, 2 of 100 and 4 of 100 on `web-transport-quinn`. `web-transport-quinn` reads the WebTransport stream header before it hands a stream out and drops a stream that is reset before the header arrives (`log_header_error`, `src/session.rs`). The matching place in `wtransport` was not read.
- **`wtransport` reads CLOSE_WEBTRANSPORT_SESSION and does not send it.** Its driver parses the capsule (`src/driver/streams/connect.rs`, lines 47–93), which is where the code 3 and `bye` above come from. `Connection::close` only closes the QUIC connection with the given code (`src/connection.rs`, lines 305–306).

Excerpt, `--library wtq --rounds 100`:

```text
[server] OBSERVED stream error=Reset(18)                                             (98 times)
[server] OBSERVED no stream surfaced, accept_uni error=WebTransportError(Closed(3, "bye"))  (2 times)
[server] OBSERVED session closed=WebTransportError(Closed(3, "bye"))                 (100 times)
[driver] RESULT {"close":{"resolved":{"closeCode":3,"reason":"protocol violation"}},"reset":{"rejected":{"name":"WebTransportError","message":"Received RESET_STREAM.","source":"stream","streamErrorCode":18}},"stop":[{"count":100,"outcome":{"rejected":{"name":"WebTransportError","message":"Received STOP_SENDING.","source":"stream","streamErrorCode":5}}}], ... ,"protocol":{"protocol":"moqt-18"},"bidi":{"replyOnOpenedStream":"pong","readOnAcceptedStream":"server-opened"},"streams":{"attempted":20,"openedWithin1s":5},"window":{"offeredBytes":1048576,"writesResolvedWithin1s":196608}}
```

Excerpt, `--library wt --rounds 20`:

```text
[server] OBSERVED stream error=Reset(91141958510829)                                 (17 times)
[server] OBSERVED accept_uni after close=Some(ApplicationClosed(ApplicationClose { code: 3, reason: [98, 121, 101] }))  (17 times)
[server] ERROR connection closed by peer: bye (code 3)                               (3 times)
[driver] RESULT {"close":{"rejected":{"name":"WebTransportError","message":"Connection lost.","source":"session","streamErrorCode":null}},"reset":{"rejected":{"name":"WebTransportError","message":"Received RESET_STREAM.","source":"stream","streamErrorCode":0}},"stop":[{"count":20,"outcome":{"rejected":{"name":"WebTransportError","message":"Received STOP_SENDING.","source":"stream","streamErrorCode":0}}}], ...}
```

### Features XmoQ uses

"Ran" means the probe passed in the runs above; the rest is from the source.

| Feature | `wtransport` 0.7.2 | `web-transport-quinn` 0.13.2 |
|---|---|---|
| Subprotocol, server | No API. The server reads `wt-available-protocols` from the request headers and writes `wt-protocol` with `accept_with_headers`, quoting by hand. Ran: page `wt.protocol` is `moqt-18` | `request.protocols` and `ConnectResponse::with_protocol`; the library refuses a protocol that was not offered (`src/connect.rs`, lines 66–70). Ran: page `wt.protocol` is `moqt-18` |
| Subprotocol, client | The caller adds the `wt-available-protocols` header. `connect` checks only the status of the response (`src/endpoint.rs`, lines 375–392), so the selected protocol cannot be read. Issue #320 asks for the response headers | `ConnectRequest::with_protocol` and `Session::protocol()`; a selected protocol that was not offered fails the connect. Ran: `protocol=Some("moqt-18")` against the moqtail relay and both probe servers |
| Client pins the server certificate by SHA-256 | `with_server_certificate_hashes`. Its documentation states the W3C rules (validity at most two weeks, ECDSA P-256); in use in `crates/moqt-client` | `ClientBuilder::with_server_certificate_hashes`. Verification stays on: the hash of the end-entity certificate is compared and the handshake signature is checked (`ServerFingerprints`, `src/client.rs`, lines 283–344). No check of validity period or key type. Ran: the right hash connects; a wrong hash fails with `invalid peer certificate: UnknownIssuer` |
| Server loads a self-signed ECDSA P-256 certificate, Chromium connects through `serverCertificateHashes` | `Identity::load_pemfiles`. Ran | No PEM loader in the library; the caller parses with `rustls-pki-types` and passes the chain and key. The SEC1 key that `openssl ecparam` writes loaded. Ran |
| Unidirectional and bidirectional streams, opened and accepted on both sides | Ran. Opening takes two awaits (`open_uni().await?.await?`) | Ran. One await |
| FIN and reset told apart | FIN is `Ok(None)`; a reset is `StreamReadError::Reset(VarInt)` with the raw QUIC code | FIN is `Ok(None)`; a reset is `ReadError::Reset(u32)`, or `InvalidReset(VarInt)` when the code is outside the WebTransport range |
| Stream limit and receive windows | `ServerConfigBuilder::with_custom_transport(identity, quinn::TransportConfig)` under the feature `quinn`; `quic_config_mut()` on a built client or server config. Ran on the server: the page opened 5 of 20 streams (HTTP/3 uses 3 of the 8) and the server received 67 kB of 1 MiB while it did not read. With defaults: 20 of 20 and 1.08 MB | The builders take only an address and a congestion controller. `Server::new(quinn::Endpoint)` and `Client::new(endpoint, quinn::ClientConfig)` take a configuration built by hand, which is what the probe server does. Ran on the server, with the same numbers as `wtransport`. A client that wants both custom transport settings and hash pinning has to bring its own certificate verifier, because `ServerFingerprints` is private. That client was not built |
| Keep-alive | `keep_alive_interval` on the builder | only through the `quinn::TransportConfig` |
| Session close with a code | `close(VarInt, &[u8])`, returns at once | `close(u32, &[u8])` writes the capsule in a spawned task; the caller awaits `closed()` for it to be delivered. The library force-closes after max(3 × RTT, 100 ms) (`src/session.rs`, lines 418 and 482) |
| Crypto backend | `ring` by default | `aws-lc-rs` by default, `ring` as a feature. With both compiled into one binary, `crypto::default_provider` panics unless a process-wide provider is installed (`src/crypto.rs`); this spike uses `ring` for both |

### Interop with moqtail and with wtransport peers

All from one run of `interop.mjs`.

| # | Client | Server | Outcome |
|---|---|---|---|
| 1 | `web-transport-quinn` client | moqtail relay 0.15.0 (`wtransport`) | connected, `protocol=Some("moqt-18")`, SETUP exchanged: the relay's SETUP carries `moqtail/0.15.0+dd8623d` |
| 2 | same, with a wrong certificate hash | moqtail relay | refused in the handshake, `UnknownIssuer` |
| 3 | `web-transport-quinn` client | probe server on `wtransport`, codes not mapped | codes cross unmapped in one direction and mapped in the other, see below |
| 4 | `web-transport-quinn` client | probe server on `web-transport-quinn` | `Closed(3, "protocol violation")`, `Reset(18)`, `Stopped(5)`; the server read `Reset(18)` and `Closed(3, "bye")` |
| 5a | moqtail Rust client (`wtransport`) | probe server on `web-transport-quinn` | SETUP exchanged in both directions; the server decoded the client's SUBSCRIBE |
| 5b | `xmoq-moqt-client` (`wtransport`) | same | SETUP exchanged in both directions; the server decoded the client's SUBSCRIBE |
| 5c | moqtail-ts page in Chromium | same | `connected: protocol=moqt-18`, SETUP sent and received, `connect()` resolved |

In 5a and 5b the client then failed, as it has to: the probe server answers no request and closes the session. Nothing beyond SETUP was exchanged with any peer.

Run 3, a `web-transport-quinn` client against a `wtransport` server that does not map:

```text
[3 wtq-client->wt-server] PROBE close: closed=ConnectionError(ApplicationClosed(ApplicationClose { error_code: 3, reason: b"protocol violation" }))
[3 wtq-client->wt-server] PROBE reset: read ended with InvalidReset(18)
[3 wtq-client->wt-server] PROBE stop: write ended with InvalidStopped(5)
[wt-server] OBSERVED stream error=Reset(91141958510829)
[wt-server] OBSERVED accept_uni after close=Some(ApplicationClosed(ApplicationClose { code: 3, reason: [98, 121, 101] }))
```

So against a `wtransport` peer such as the moqtail relay, a `web-transport-quinn` endpoint receives the peer's stream codes as `InvalidReset` and `InvalidStopped` with the raw value inside, and the peer receives the mapped value where it expects a MOQT code. The session close code arrives in both directions, in different error variants.

### Maintenance

Read from crates.io and GitHub on 2026-10-10.

| Item | `wtransport` | `web-transport-quinn` |
|---|---|---|
| Repository | `BiagioFesta/wtransport` | `moq-dev/web-transport` |
| Latest release | 0.7.2, 2026-08-11 | 0.13.2, 2026-10-07 |
| Releases before it | 0.7.1 on 2026-04-26, 0.7.0 on 2026-02-08; 25 versions since 2023-05 | 0.13.1 on 2026-10-02, 0.13.0 on 2026-09-27, 0.12.1 on 2026-08-20, 0.12.0 on 2026-08-07; 42 versions since 2024-04 |
| Latest commit on the default branch | `b16674e`, 2026-09-22 | `d4e4e45`, 2026-10-09 |
| License | MIT OR Apache-2.0 | MIT OR Apache-2.0 |
| `quinn`, `rustls` requirement | `0.11.6`, `0.23.23` | `0.11`, `0.23` |
| Resolved here | `quinn` 0.11.12, `rustls` 0.23.45 | the same crates |
| Declared Rust version | 1.88 | none; built here with 1.99.0 |
| Crates in the repository | 2 (`wtransport`, `wtransport-proto`) | 11; this one pulls `web-transport-proto`, `web-transport-trait` and `kio` (from `moq-dev/moq`) |
| Crates in the dependency tree, normal and build, on this host, fresh resolution | 116 | 112 with `aws-lc-rs` (builds `aws-lc-sys`), 107 with `ring` |
| HTTP/3 layer | own (`wtransport-proto`) | own (`web-transport-proto`) |
| Sessions per QUIC connection | not checked | one; the session owns the connection (crate documentation, `src/lib.rs`) |
| Open issues and pull requests (GitHub count) | 38 | 16 |

### Outlook for wtransport upstream

- **Error code mapping:** issue #182 "Wrong model for stream error codes", open since 2024-05-09. Pull request #211 "Convert error codes before passing them to QUIC", open since 2024-09-03, last activity 2024-09-14, now in conflict with `master`. The maintainer asked one question on it.
- **Sending CLOSE_WEBTRANSPORT_SESSION:** no issue or pull request was found (searched for `capsule`, `CLOSE_WEBTRANSPORT_SESSION`, `close` in titles). Issue #242, open since 2024-12-03, says session termination was added in pull request #239 and asks for the send sides to be reset when a session is terminated.
- **Newer WebTransport drafts:** issues #328 (accept `:protocol` = `webtransport-h3`, 2026-09-25) and #332 (send SETTINGS_WT_ENABLED, 2026-10-01) are open without comments. `web-transport-proto` 0.6.2 also accepts only `webtransport` (`src/connect.rs`, lines 161–163). Chromium 156 connected to both libraries.

### What each choice would need in XmoQ

As far as these runs and the sources show. Neither list was carried out.

Switching to `web-transport-quinn`:

- Port `crates/moqt-client/src/io.rs` and the close in `main.rs`: connect through `ClientBuilder` and `ConnectRequest::with_protocol`, one await to open a stream, `finish()` without await, `stop` and `reset` with `u32` codes, `ReadError::Reset(u32)` next to `InvalidReset(VarInt)`, and `closed()` awaited after `close` so that the capsule leaves.
- Check again what the comments in `io.rs` state about `wtransport` and `quinn` (what dropping a stream sends, the default receive window).
- Pick the crypto backend: `ring`, as now, or `aws-lc-rs`.
- For transport limits on the client, write a certificate verifier for the pinned hash, or go without limits there.
- Decide how stream codes are read from and sent to `wtransport` peers, the moqtail relay and clients among them, which stay the interop peers.
- Repeat the seven runs of the [own client record](2026-10-10-own-rust-client-vs-moqtail.md), and update `docs/protocols.md`.
- The relay being written on `wtransport` changes with it.

Staying on `wtransport` 0.7.2:

- A browser cannot learn the code or reason of a session the relay closes; `WebTransport.closed` rejects with `Connection lost.`. The library has no call that sends the capsule.
- XmoQ maps stream codes itself, before `reset` and `stop` and after a received reset or STOP_SENDING. The sending direction ran here; the pull request that would move it into the library has not moved since 2024-09.
- The subprotocol stays hand-written headers, and the client cannot check which protocol the server selected.
- The same question about `wtransport` peers that do not map applies, once XmoQ maps.

## Limitations and blockers

No blocker.

- Chromium only, headless, one machine, loopback. No other browser, no packet loss.
- Why a STOP_SENDING sometimes reaches the page without a code was not investigated, and neither was why that never happened with the limits of this spike. The counts are small.
- The moqtail peers were taken to the SETUP exchange only. No Object was sent over `web-transport-quinn`, and XmoQ's client was not ported to it.
- How the moqtail relay reacts to a mapped stream code from a `web-transport-quinn` client was not run.
- `web-transport-quinn` ran with `ring`. Its default backend `aws-lc-rs` was resolved for the crate count and not run.
- A `web-transport-quinn` client with custom transport settings was not built. The limits were checked on the server side, from the browser.
- Datagrams, several sessions on one connection, connection migration and throughput were not looked at.
- The `wtransport` column for the page probes is a re-run with this spike's server, not a copy of the `wt-error-codes` results. Where the two differ (the STOP_SENDING rate) it says so above.
- What dropping a stream does (FIN, STOP_SENDING code) was not checked for `web-transport-quinn`. The comments in `crates/moqt-client/src/io.rs` state it for `wtransport`.

## Open questions raised

1. **Which library does the own relay build on?** The two lists above say what each choice needs.
2. **Does XmoQ need the session termination code (§3.5) to reach a browser?** With `wtransport` 0.7.2 it cannot.
3. **How are stream codes handled against `wtransport` peers that do not map**, the moqtail relay among them? This holds for either library as soon as XmoQ maps its own codes.
4. **Is a STOP_SENDING or reset whose code does not arrive treated like an unknown code?** It happened in about 1 of 100 rounds on both libraries.
