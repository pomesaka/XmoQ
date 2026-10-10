# MoQ library candidates

Status: **survey, nothing selected.** The library pair and the wire protocol are still open decisions (see [Open questions](open-questions.md)). The survey itself is from documentation and source reading. The moq-dev pair was then run locally; see [the experiment record](experiments/2026-10-10-moq-dev-object-transfer.md). Statements changed by that run are marked "(run 2026-10-10)".

All facts below were checked on **2026-10-10** against the sources in [Sources](#sources). `[n]` refers to that list. "Last commit" is the default branch unless noted.

## Two different protocols are called "MoQ"

| Protocol | Document | Status |
|---|---|---|
| MoQ Transport (MOQT) | `draft-ietf-moq-transport-22`, published 2026-10-01, expires 2027-04-04 | IETF MoQ WG document, not yet an RFC [1] |
| moq-lite | `draft-lcurley-moq-lite-06`, published 2026-09-24 | Individual draft, not a WG document [2] |

moq-lite is described by its author as "a forwards-compatible subset" of MOQT [3]. It has its own wire format and its own ALPN (`moq-lite-06`), so a moq-lite session is not an MOQT session. Whether moq-lite satisfies XmoQ's "uses MoQ" requirement is a decision for the project owner.

MOQT drafts are not wire compatible with each other. Draft 15 and later select the version by ALPN / WebTransport subprotocol (`moqt-NN`); draft 14 uses `moq-00` plus SETUP negotiation [4].

## Candidates

### Rust

| Candidate | Protocol and versions | Relay | WebTransport stack | Latest release | Last commit | License |
|---|---|---|---|---|---|---|
| [moq-dev/moq](https://github.com/moq-dev/moq) (`moq-net`, `moq-relay`, `moq-cli`; formerly `kixelated/moq-rs`) | moq-lite 01–06 **and** MOQT drafts 14–22, negotiated per session. moq-lite is preferred by default [4] | Yes (`moq-relay`) | `web-transport-moq` on the project's own QUIC fork `moq-noq`: 2.0.2 on `main` [5], 1.3.4 in the released `moq-tokio` 0.19.23 (run 2026-10-10) | `moq-relay` 0.17.2 (2026-10-06), `moq-net` 0.3.10 on crates.io [6] | 2026-10-09 | MIT OR Apache-2.0 |
| [moqtail/moqtail](https://github.com/moqtail/moqtail) (`moqtail` crate, `relay`) | MOQT only. Release 0.15.0 = draft 18. `main` = draft 22, marked work in progress [7] | Yes | `wtransport` 0.7.2 (quinn) [8] | `relay@0.15.0`, `moqtail-rs@0.15.0` (2026-10-04) | 2026-10-08 | Apache-2.0 |
| [cloudflare/moq-rs](https://github.com/cloudflare/moq-rs) (`moq-transport`, `moq-relay-ietf`; formerly `englishm/moq-rs`) | MOQT only. `main` = draft 16. Branches for draft 14 and draft 18 (dev) [9] | Yes | `web-transport-quinn` 0.11.8, `quinn` 0.11.9 [10] | `moq-transport` 0.16.4 (2026-10-06) | 2026-10-06 | MIT OR Apache-2.0 |
| [nttcom/moq-wasm](https://github.com/nttcom/moq-wasm) (`moqt`, `relay`) | MOQT draft 14 [11] | Yes | `web-transport-quinn` 0.11 | No release or tag | 2026-10-09 | MIT |
| [shiguredo/moqt-rs](https://github.com/shiguredo/moqt-rs) | MOQT draft 22, sans-I/O codec and session state machine [12] | **No** ("relay は別途用意") | None in the library | No release | 2026-10-09 (last push) | Apache-2.0 |

### Browser (TypeScript, WebTransport)

| Candidate | Protocol and versions | Publish / subscribe | Latest release | Last commit | License |
|---|---|---|---|---|---|
| `@moq/net` in [moq-dev/moq](https://github.com/moq-dev/moq) (successor of the archived `kixelated/moq-js`) | moq-lite 01–06 and MOQT drafts 14–22 [13] | Both | 0.4.2 on npm (2026-10-04) | 2026-10-09 | MIT OR Apache-2.0 |
| `moqtail` (moqtail-ts) in [moqtail/moqtail](https://github.com/moqtail/moqtail) | MOQT only. npm 0.13.0 = draft 18. `main` = draft 22, marked work in progress [7] | Both | 0.13.0 on npm (2026-10-04) | 2026-10-08 | Apache-2.0 |
| `@openmoq/*` in [openmoq/moq-playa](https://github.com/openmoq/moq-playa) | MOQT drafts 14, 16 (default), 18, 22 (opt-in) [14] | Player first; a publisher example exists | Tag `v0.5.9`, "pre-release" | 2026-10-09 (last push) | Apache-2.0 |
| [shiguredo/moqt-js](https://github.com/shiguredo/moqt-js) | MOQT draft 22 [15] | Both | 2026.2.0 (2026-05-13) | 2026-10-09 (last push) | Apache-2.0 |
| [video-dev/moq-js](https://github.com/video-dev/moq-js) | MOQT draft 14 only [16] | Player | v0.4.3 (2025-03-11) | 2026-06-26 (last push) | MIT OR Apache-2.0 |
| `crates/wasm` in [nttcom/moq-wasm](https://github.com/nttcom/moq-wasm) | MOQT draft 14, Rust compiled to WebAssembly [11] | Both | No release | 2026-10-09 | MIT |

### Local connection with a self-signed certificate

| Candidate | Documented method |
|---|---|
| moq-dev/moq | The relay serves `GET /certificate.sha256`. Given an `http://` URL, `@moq/net` fetches that fingerprint and pins it through `serverCertificateHashes`. A hash or PEM can also be passed directly [17] |
| moqtail | `mkcert`, then enable `chrome://flags/#webtransport-developer-mode` (Chrome) or set an `about:config` flag (Firefox) [18] |
| cloudflare/moq-rs | `./dev/relay` generates certificates with `mkcert`. No browser steps in this repository; it points to `video-dev/moq-js` for playback [19] |
| nttcom/moq-wasm | `make chrome` launches Chrome trusting the relay certificate [11] |
| openmoq/moq-playa | `certHash` option (SHA-256) [14] |

## Pairs that share a version

"Tested" means a first-party record exists. "On paper" means the version numbers match and nothing more.

| Rust side | Browser side | Shared versions | Evidence |
|---|---|---|---|
| moq-dev `moq-relay` / `moq-net` | `@moq/net` | moq-lite 01–06, MOQT 14–22 | **Tested, same author.** Nightly in-tree matrix runs headless Chromium against `moq-relay`, plus byte-level Rust↔JS codec checks for MOQT datagrams on drafts 14–22 [20] |
| moqtail `relay` 0.15.0 | `moqtail` npm 0.13.0 | MOQT 18 | Same author, same repository. Interop record: unconfirmed |
| moqtail `main` | moqtail-ts `main` | MOQT 22 | Same author. Marked work in progress [7] |
| nttcom `relay` | nttcom wasm | MOQT 14 | Same author, browser E2E target exists [11] |
| cloudflare `moq-relay-ietf` (`main`) | `@moq/net`, `@openmoq/*` | MOQT 16 | On paper |
| cloudflare `moq-relay-ietf` (draft-14 branch) | `video-dev/moq-js`, `@moq/net`, `@openmoq/*` | MOQT 14 | cloudflare/moq-rs names `video-dev/moq-js` as its player [9]; no test record found |
| moqtail `relay` 0.15.0 | `@moq/net`, `@openmoq/*` | MOQT 18 | On paper |
| moq-dev `moq-relay` | `@openmoq/*`, moqtail-ts, shiguredo/moqt-js | MOQT 14–22 where each client supports it | On paper |
| shiguredo/moqt-rs | shiguredo/moqt-js | MOQT 22 | No relay in either repository |

Cross-implementation interop is tracked by [englishm/moq-interop-runner](https://github.com/englishm/moq-interop-runner), whose registry lists `moq-rs`, `moq (moq-dev)`, `moqtail`, `moq-playa` and others with a current target of draft 18. It explicitly excludes browser-only clients; JS clients run under Bun or Node with a WebTransport polyfill [21]. Published pass/fail results were not located.

## Recommended pair

**`moq-dev/moq`: `moq-relay` 0.17.2 and `moq-net` 0.3.10 (Rust) with `@moq/net` 0.4.2 (browser).**

- It is the only candidate where one project ships a Rust relay, a Rust client library and a browser library, and tests the three together in a real browser on every nightly run [20].
- The same pair speaks both moq-lite and MOQT drafts 14–22, so the protocol decision does not force a library change [4][13].
- The self-signed local flow needs no browser flags [17].
- Phase 3 needs a Rust subscriber; `moq-net` is the library the relay itself is built on.

Version to pin depends on the open decision below:

- If moq-lite is acceptable: `moq-lite-06` (the default both sides offer first).
- If the wire must be IETF MOQT: pin an MOQT ALPN on both sides. Do not treat `moqt-22` on these versions as draft 22: both libraries still send the draft-20/21 `LOCATION_FILTER` under that ALPN (run 2026-10-10).

Pinning is `--listen-version` on the relay, `--connect-version` on a `moq-tokio` client and `webtransport: { protocols: [...] }` in `@moq/net`. All three were exercised (run 2026-10-10).

Result of the local run: browser ↔ Rust passed in both directions on moq-lite-06 and on ALPNs for drafts 16, 18, 20 and 22. Browser to browser failed in most runs when the subscriber joined after the object was written, on every version, and passed when the publisher kept writing.

Known limits of this pair in MOQT mode, from its own documentation [22]: the implementation keeps moq-lite's model and refuses or fakes the rest. FETCH returns one group, single-track `PUBLISH` is declined, one publisher per path, unknown object properties are dropped. Three behaviours are listed as deliberate deviations from the draft.

If strict MOQT conformance matters more than tested browser interop, the alternative is **moqtail** at draft 18 (`relay@0.15.0`, `moqtail` crate 0.15.0, `moqtail` npm 0.13.0).

## Not recommended for Phase 0

| Candidate | Reason |
|---|---|
| cloudflare/moq-rs | No browser library in the project. The named player, `video-dev/moq-js`, speaks draft 14 only and was last pushed 2026-06-26, while `main` is draft 16 |
| nttcom/moq-wasm | Draft 14 only; no release or tag to pin |
| shiguredo/moqt-rs + moqt-js | No relay. moqt-js states it is tested only against the vendor's own Sora MoQ and will not be tested against other relays [15] |
| openmoq/moq-playa | Browser side only, pre-release. Useful as a second MOQT client for cross-checks |
| kixelated/moq-js | Archived; last push 2025-06-20 |
| moxygen (C++), imquic (C), libquicr (C++), quiche (C++), gomoqt / moqtransport (Go) | Not Rust. Their public relays are possible interop targets [21] |

## Cloudflare's hosted relay

Cloudflare runs MOQT relays built from `cloudflare/moq-rs` [9][23].

- Versions: draft 14 and draft 16. Draft 18 is "not yet supported"; an interop-only draft-18 endpoint is listed in the interop runner registry [21][23].
- Endpoints: `https://draft-14.cloudflare.mediaoverquic.com/`, `https://draft-16.cloudflare.mediaoverquic.com/<token>`.
- Free during beta. A relay is provisioned from the Cloudflare dashboard or API. Draft 16 requires a token in the URL path.
- Not supported on either draft: FETCH, GOAWAY, SUBSCRIBE_UPDATE [24].

It can serve as a comparison target for a browser client on draft 14 or 16. It does not remove the need for Rust code in Phase 0, because the roadmap gate is browser ↔ Rust. Whether it replaces a self-hosted relay later is a hosting decision, which is open.

## Browser support for WebTransport

From MDN browser-compat-data [25]. Supported browsers for XmoQ are undecided.

| Feature | Chrome | Firefox | Safari |
|---|---|---|---|
| `WebTransport` | 97 | 114 | 26.4 |
| `serverCertificateHashes` | 100 | 125 | 26.4 |
| `protocols` option and `protocol` property | 143 | 155 | 26.4 |

MOQT draft 15 and later, and moq-lite 03 and later, are selected through `protocols`. `@moq/net` falls back to draft 14 or moq-lite 01/02 SETUP negotiation when the browser reports no subprotocol [13].

## Decisions needed

1. **MOQT or moq-lite on the wire.** The recommended pair supports both.
2. **Which MOQT draft to pin**, if MOQT. Draft 22 is current; draft 18 is the interop runner's target; Cloudflare's hosted relay stops at draft 16. On the moq-dev pair, the newest draft whose wire format is actually implemented is 21 (run 2026-10-10).
3. **Whether the moq-lite-model limits in MOQT mode are acceptable** [22], or strict MOQT (moqtail) is required.

## Unconfirmed

- Only the moq-dev pair was executed. Every other pairing is from documentation and source reading.
- Whether `moq-dev/moq` `main` implements the draft-22 `LOCATION_FILTER` on the JS side. The released `@moq/net` 0.4.2 and `moq-net` 0.3.10 do not; the Rust side of `main` has a test for it.
- The cause of the browser-to-browser late-join failure in `@moq/net` 0.4.2, and whether it is fixed on `main`.
- Published pass/fail results from moq-interop-runner, and any cross-implementation result that involves a real browser.
- moqtail: interop test records between its relay and moqtail-ts; which draft the `ghcr.io/moqtail/relay:latest` image speaks.
- `@openmoq/*` package availability and version on npm (the registry lookup failed).
- Lineage and maintenance of `moq-noq` / `web-transport-moq` beyond the comment that it is the project's fork of `noq`.
- Cloudflare relay: datagram support, raw QUIC versus WebTransport details, and limits after beta.
- Firefox and Safari behaviour with any candidate. moq-dev's browser tests cover Chromium only [20].

## Sources

1. Datatracker API, `draft-ietf-moq-transport`: <https://datatracker.ietf.org/api/v1/doc/document/?name=draft-ietf-moq-transport> (rev 22, time 2026-10-01)
2. Datatracker API, `draft-lcurley-moq-lite`: <https://datatracker.ietf.org/api/v1/doc/document/?name=draft-lcurley-moq-lite> (rev 06, no stream)
3. `moq-dev/moq` `README.md`, `doc/concept/moq-lite.md` @ `33d09af`
4. `moq-dev/moq` `rs/moq-net/src/version.rs` @ `33d09af`
5. `moq-dev/moq` `Cargo.toml`, `rs/moq-tokio/Cargo.toml` @ `33d09af`
6. `moq-dev/moq` GitHub releases; <https://crates.io/api/v1/crates/moq-net>
7. `moqtail/moqtail` `README.md` @ `ee9753c`; `libs/moqtail-rs/src/model/control/constant.rs` at tag `relay@0.15.0` (`moqt-18`) and on `main` (`moqt-22`); `libs/moqtail-ts/src/model/control/constant.ts` at tag `moqtail@0.13.0` (`moqt-18`)
8. `moqtail/moqtail` `Cargo.toml` @ `ee9753c`
9. `cloudflare/moq-rs` `README.md` @ `4df0e9f`
10. `cloudflare/moq-rs` `moq-native-ietf/Cargo.toml` @ `4df0e9f`
11. `nttcom/moq-wasm` `README.md`, `crates/moqt/Cargo.toml`, `Makefile` @ `b93dcdc`
12. `shiguredo/moqt-rs` `README.md` (default branch)
13. `moq-dev/moq` `js/net/src/ietf/version.ts`, `js/net/src/connection/connect.ts`, `js/net/package.json` @ `33d09af`; <https://registry.npmjs.org/@moq%2Fnet>
14. `openmoq/moq-playa` `README.md` (default branch)
15. `shiguredo/moqt-js` `README.md` (default branch)
16. `video-dev/moq-js` `lib/transport/client.ts` (default branch)
17. `moq-dev/moq` `doc/bin/relay/http.md`, `js/net/src/connection/connect.ts` @ `33d09af`
18. `moqtail/moqtail` `apps/relay/cert/README.md` @ `ee9753c`
19. `cloudflare/moq-rs` `dev/README.md` @ `4df0e9f`
20. `moq-dev/moq` `test/interop/README.md` @ `33d09af`
21. `englishm/moq-interop-runner` `README.md`, `implementations.json` @ `a4c554e`
22. `moq-dev/moq` `doc/concept/standard.md` @ `33d09af`
23. <https://developers.cloudflare.com/moq/> (page dated 2026-07-31)
24. <https://developers.cloudflare.com/moq/feature-matrix/> (page dated 2026-07-31)
25. `mdn/browser-compat-data` `api/WebTransport.json` @ `0313647` (2026-10-07)
