# 2026-10-10 — moqtail: one Object between browser and Rust (MOQT draft 18)

## Objective

Phase 0 gate from [the roadmap](../roadmap.md): transfer one MoQ Object between a real Chromium and a Rust client through a relay, using moqtail pinned to MOQT draft 18, and check that the received payload equals the sent one.

## Result

**Passed.** One Object went through the moqtail relay in all three directions tried, and the received bytes matched the sent bytes each time.

| # | Direction | Browser | Certificate trust | Outcome |
|---|---|---|---|---|
| 1 | browser → Rust | Chromium 156.0.8078.4 (new headless) | `serverCertificateHashes` | payload match |
| 2 | Rust → browser | Chromium 156.0.8078.4 (new headless) | `serverCertificateHashes` | payload match |
| 3 | browser → browser | Chromium 156.0.8078.4 (new headless) | `serverCertificateHashes` | payload match |
| 4 | browser → Rust | chrome-headless-shell 156.0.8078.4 | `serverCertificateHashes` | payload match |
| 5 | browser → Rust | Google Chrome 155.0.8059.39 (headless) | `serverCertificateHashes` | payload match |
| 6 | browser → Rust | Chromium 156.0.8078.4 (new headless) | launch arguments | payload match |
| 7 | browser → Rust | Chromium 156.0.8078.4 (new headless) | none (negative control) | fails in the TLS handshake, as expected |

Runs 1–3 are from the final state of the code, after `npm ci`. Runs 4–7 were made just before two last changes (adding the `none` value to an option list, and reinstalling with `npm ci`) and were not repeated.

## Environment and versions

| Item | Value |
|---|---|
| XmoQ commit | working copy on top of `af05a7d` (uncommitted) |
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Relay | `relay` 0.15.0, reports `moqtail/0.15.0+dd8623d`. Built from `moqtail/moqtail` tag `relay@0.15.0` (commit `dd8623d`) with `cargo install --locked`, so upstream's `Cargo.lock` applies (`wtransport` 0.7.2, `quinn` 0.11.9, `rustls` 0.23.36) |
| Rust client | `moqtail` 0.15.0 and `wtransport` 0.7.2 from crates.io, `tokio` 1.49.0. Its own lockfile resolved `quinn` 0.11.12 and `rustls` 0.23.45 |
| Browser library | `moqtail` 0.13.0 from npm, bundled with `esbuild` 0.28.2 |
| Browser driver | `playwright` 1.64.0 |
| Toolchain | `cargo` 1.92.0, `node` 23.10.0, `npm` 10.9.2, `typescript` 6.0.3, LibreSSL 3.3.6 as `openssl` |
| Wire | MOQT draft 18. WebTransport subprotocol `moqt-18` on every session |

The draft was checked in the artifacts actually used, not only in the repository: `SUPPORTED_VERSIONS` is `"moqt-18"` in the crates.io source of `moqtail` 0.15.0, and `moqt-18` is the only version string in the npm package's `dist/model.mjs`.

## Commands

From the repository root. Each command below was run as written, except that absolute paths were used.

```sh
cargo install --git https://github.com/moqtail/moqtail --tag relay@0.15.0 --locked --root spikes/moqtail/.tools relay
cargo build --locked --manifest-path spikes/moqtail/rust-client/Cargo.toml
npm --prefix spikes/moqtail/browser ci
npm --prefix spikes/moqtail/browser exec -- playwright install chromium
npm --prefix spikes/moqtail/browser run typecheck

node spikes/moqtail/browser/run.mjs --direction browser-to-rust                         # run 1
node spikes/moqtail/browser/run.mjs --direction rust-to-browser                         # run 2
node spikes/moqtail/browser/run.mjs --direction browser-to-browser                      # run 3
node spikes/moqtail/browser/run.mjs --direction browser-to-rust --browser headless-shell  # run 4
node spikes/moqtail/browser/run.mjs --direction browser-to-rust --browser chrome          # run 5
node spikes/moqtail/browser/run.mjs --direction browser-to-rust --cert-trust spki-flag    # run 6
node spikes/moqtail/browser/run.mjs --direction browser-to-rust --cert-trust none         # run 7
```

A different `--port` (44318–44324) was passed per run. `run.mjs` starts the relay, the page server, the browser and the Rust client, and stops them when it ends. The code is in [`spikes/moqtail/`](../../spikes/moqtail/README.md).

## What each side does

- **Browser publisher**: `MOQtailClient.new` with `transportOptions.serverCertificateHashes`, `publishNamespace`, `addOrUpdateTrack` with a `LiveTrackSource`, `publish`, then one `MoqtObject` (group 0, object 0, subgroup forwarding) enqueued after the subscriber is live.
- **Browser subscriber**: `client.subscribe` with `FilterType.LatestObject`, then reads the first Object with a payload from the returned stream.
- **Rust subscriber**: `wtransport` connect with the `wt-available-protocols: "moqt-18"` header, SETUP on a pair of unidirectional streams, SUBSCRIBE on a bidirectional stream, then `RecvDataStream::next_object` on the first incoming unidirectional stream.
- **Rust publisher**: same session setup, PUBLISH on a bidirectional stream, then one Object on a subgroup stream.

The payload is 53 bytes of text with a timestamp and a random nonce, generated per run.

## Observed output

Run 1, browser → Rust (relay lines shortened, most relay lines omitted):

```text
[relay] relay::server: moqtail/0.15.0+dd8623d is running on 1 UDP socket(s) -- WebTransport clients: https://127.0.0.1:44318  Raw QUIC clients: moqt://127.0.0.1:44318
[driver] browser=chromium version=156.0.8078.4 args=[]
[relay] Selected protocol version: moqt-18
[relay] control message received from conn 5217828368 — Setup (4 bytes): af 00 00 00
[relay] control message sending to conn 5217828368 — Setup (28 bytes): af 00 00 18 07 16 6d 6f 71 74 61 69 6c 2f 30 2e 31 35 2e 30 2b 64 64 38 36 32 33 64
[browser-pub] connected: protocol=moqt-18
[relay] control message received from conn 5217828368 — PublishNamespace (18 bytes): 06 00 0f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 00
[relay] control message sending to conn 5217828368 — RequestOk (4 bytes): 07 00 01 00
[relay] control message received from conn 5217828368 — Publish (36 bytes): 1d 00 21 02 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 ff 0a d2 0b 51 39 c4 21 d5 01 10 01
[relay] control message sending to conn 5217828368 — RequestOk (8 bytes): 07 00 05 02 10 01 12 01
[relay] Selected protocol version: moqt-18
[rust-sub] SETUP_OK server_setup=Setup { setup_options: [Bytes { type_value: 7, value: b"moqtail/0.15.0+dd8623d" }] }
[relay] control message received from conn 5217875984 — Subscribe (34 bytes): 03 00 1f 00 02 04 78 6d 6f 71 06 70 68 61 73 65 30 06 6f 62 6a 65 63 74 04 10 01 10 80 01 01 02 01 01
[relay] control message sending to conn 5217875984 — SubscribeOk (5 bytes): 04 00 02 00 00
[rust-sub] SUBSCRIBED track_alias=0
[browser-pub] object enqueued: group=0 object=0 payload_len=53
[relay] new group: relay_track_id=0 location: Location { group: 0, object: 0 } stream_id=subgroup_0_0_0 time=0
[rust-sub] OBJECT group=0 object=0 status=Normal payload_len=53 payload_hex=786d6f712d70686173653020323032362d31302d31305430303a33363a33302e3139365a2037613961343234623166323638393934
[driver] RESULT {"direction":"browser-to-rust","match":true,"sentText":"xmoq-phase0 2026-10-10T00:36:30.196Z 7a9a424b1f268994","receivedText":"xmoq-phase0 2026-10-10T00:36:30.196Z 7a9a424b1f268994", ... ,"relayVersion":"moqtail/0.15.0+dd8623d","browser":"chromium","browserVersion":"156.0.8078.4","certTrust":"hash"}
```

Runs 2 and 3 ended with:

```text
[driver] RESULT {"direction":"rust-to-browser","match":true,"sentText":"xmoq-phase0 2026-10-10T00:36:31.402Z f6ebf924865b83bd","receivedText":"xmoq-phase0 2026-10-10T00:36:31.402Z f6ebf924865b83bd", ...}
[driver] RESULT {"direction":"browser-to-browser","match":true,"sentText":"xmoq-phase0 2026-10-10T00:36:32.228Z fe526d3022a29cfd","receivedText":"xmoq-phase0 2026-10-10T00:36:32.228Z fe526d3022a29cfd", ...}
```

Run 7, with no trust configured, stopped at the certificate:

```text
[relay] ERROR relay::server: Error occurred in session 0: connection aborted by peer: the cryptographic handshake failed: error 46: ... certificate unknown ...
[browser-pub] Failed to establish a connection to https://127.0.0.1:44324/: net::ERR_QUIC_PROTOCOL_ERROR.QUIC_TLS_CERTIFICATE_UNKNOWN (...)
[driver] FAILED page.evaluate: WebTransportError: Opening handshake failed.
```

## Browser settings that were needed

- **No browser flag and no root CA** with `serverCertificateHashes`. `moqtail` 0.13.0 forwards `transportOptions` to the `WebTransport` constructor, so the hash can be passed through `MOQtailClient.new`. The certificate has to be ECDSA P-256 and valid for under two weeks; the spike uses 13 days.
- **Launch arguments also work** (run 6): `--ignore-certificate-errors-spki-list=<base64 SHA-256 of the SPKI>` together with `--origin-to-force-quic-on=127.0.0.1:<port>`, passed through Playwright's `args`. The page then needs no certificate option.
- The page is served from `http://localhost:<port>`, a secure context, by a throwaway HTTP server.
- Version selection uses the `protocols` option of `WebTransport`. Chromium 156 and Chrome 155 both reported `protocol=moqt-18`.

## Problems met on the way

- **Private key format.** `openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1` under LibreSSL 3.3.6 produced a key the relay could not load; it panicked in `wtransport-0.7.2/src/tls.rs:652` with `failed to parse private key as RSA, ECDSA, or EdDSA`. Generating the key with `openssl ecparam -name prime256v1 -genkey -noout` and the certificate with `openssl req -x509 -new -key` fixed it.
- **TypeScript types.** `typescript` 6.0.3's DOM library has no `WebTransport.protocol` (TS2339). The spike reads it with `Reflect.get`.

## Differences from the library survey

Compared with `docs/moq-libraries.md` (in another workspace, not edited here):

| Survey | Found |
|---|---|
| Local connection for moqtail needs `mkcert` plus `chrome://flags/#webtransport-developer-mode` | Neither is needed. `serverCertificateHashes` works through `MOQtailClient.new`, and so do the two launch arguments above. The `mkcert` route was not run |
| Interop record between the relay and moqtail-ts: unconfirmed | No browser or end-to-end job exists in `.github/workflows/` at tag `relay@0.15.0`. This run is a record for Chromium 156 and Chrome 155 |
| Draft spoken by `ghcr.io/moqtail/relay:latest`: unconfirmed | `.github/workflows/relay.yml` at the tag applies `latest` only on pushes of `relay@*` tags, so `latest` follows the newest relay release. The image itself was not pulled |
| `moqtail` crate 0.15.0 is draft 18 | Confirmed in the source and by this run. Its crates.io description still says "Draft 16-compliant" |

Also relevant to later phases, not stated in the survey:

- The `moqtail` crate has **no client API**. `libs/moqtail-rs/src/client.rs` contains only a license header. A Rust client has to open the control streams, exchange SETUP and drive request streams itself from the `model` and `transport` modules, as upstream's `apps/client` does. The spike's Rust client is about 300 lines for that reason.
- The `libs/moqtail-ts/README.md` examples do not match the published API (for example `MOQtailClient.new(clientSetup, webTransport)` and `LiveContentSource`, where 0.13.0 has `MOQtailClient.new({ url, ... })` and `LiveTrackSource`). `apps/client-js` in the same repository was the usable reference.

## Other observations

- The relay rewrites track aliases: in run 1 the browser published with alias `779698128977928661`, and the Rust subscriber received `track_alias=0`.
- `onMessageSent` and `onMessageReceived` of moqtail-ts fired only for SETUP. Messages on request streams (PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE and their responses) did not reach these callbacks.
- The relay binds `[::]:<port>` dual-stack whatever `--host` is; `--host` only appears in its startup log line (`apps/relay/src/server/config.rs` at the tag). Remote addresses were logged as `[::ffff:127.0.0.1]`.
- The relay logs `ERROR ... Failed to accept bi-stream: ApplicationClosed` many times when a client closes its session. In every run these lines came after the Object was delivered.

## Provisional settings

Chosen only to make this check run. None of them is a project decision.

- No authentication or authorization on the relay.
- Self-signed certificate, 13-day validity, pinned by hash on both clients.
- Namespace `xmoq/phase0`, track `object`, one Object at group 0 / object 0, subscriber priority 128.
- Relay options left at defaults except port, certificate paths and `--log-folder spikes/moqtail/.logs`.
- Chromium only; the supported browser set is still undecided.

## Limitations and unconfirmed points

- One Object on one subgroup stream. Not covered: several Objects or groups, datagrams, FETCH, late join, reconnect, cancellation, slow subscribers, backpressure.
- The subscriber was always live before the Object was sent, because the subscription uses the `LatestObject` filter.
- The browser publisher used PUBLISH after PUBLISH_NAMESPACE, as `apps/client-js` does. The path where the relay forwards a SUBSCRIBE to a publisher that only announced a namespace was not tried.
- Only headless browsers on one macOS arm64 machine over loopback. Headed Chrome, Firefox, Safari, another OS and a real network were not tried.
- The `mkcert` plus `webtransport-developer-mode` route was not run: `mkcert` is not installed, and `mkcert -install` changes the system trust store.
- A certificate issued by a public CA, which a deployment would use instead of a 13-day pinned one, was not tried.
- Raw QUIC (`moqt://`, ALPN `moqt-18`) was not tried; all sessions were WebTransport.
- The relay's 403 response to a client that offers no `moqt-18` subprotocol was read in the source, not triggered.
- Interop with a non-moqtail implementation at draft 18 was not tried.

## Blockers

None for the Phase 0 gate with moqtail on Chromium.
