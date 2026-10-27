# moqlivemock as an interop peer

[`Eyevinn/moqlivemock`](https://github.com/Eyevinn/moqlivemock) v0.16.1 (commit `91d903e9933a452f8baaa860f9fcd20cf195e077`), a publisher and a subscriber for draft 18 of MoQ Transport that are not ours, with an MSF catalog and LOC media. None of its source is kept here: `scripts/ci/interop.sh` builds two commands from the tag into `.tools/`, which is not committed.

| In `.tools/` | What it is here |
|---|---|
| `bin/mlmpub` | The publisher, which is a server: a client connects to it, and no relay is in between. `interop/catalog.ts` has an own page read its catalog |
| `bin/mlmsub` | The subscriber, **built with a patch of one line** (below). `interop/catalog.ts` has it read a catalog an own page publishes through the own relay. `-catalogout <file>` makes it write the catalog it read, which the run compares |
| `assets/test10s` | The media `mlmpub` serves, copied from the tag: 7.6 MB. Its `-asset` flag defaults to a path relative to its source, so the run passes this one |

## Build

`mise run interop` does it, and fails when the tag is no longer the commit above. The steps, and why not `go install`, are in `scripts/ci/interop.sh` alone.

## The patch to `mlmsub`

`mlmsub-legacy-connect-protocol.patch` adds one line to `cmd/mlmsub/handler.go`: `LegacyConnectProtocol: true` in the settings of the WebTransport dialer. **`mlmpub` is built as it is.**

- **What it changes:** the `:protocol` of the CONNECT request that opens the WebTransport session. The library moqlivemock uses (`Eyevinn/webtransport-go`, a fork of `quic-go/webtransport-go` 0.12.0) sends `webtransport-h3`, the token of `draft-ietf-webtrans-http3-15`, and with that setting `webtransport`, the token before it. `mlmsub` has no flag for it. Nothing of MoQ Transport or of MSF is touched.
- **Without it, `mlmsub` connects to neither relay here** (seen with the binary of the tag):
  - the own relay prints `HANDSHAKE_FAILED connection lost: failed to exchange h3 connect: protocol error: expected webtransport, got: Some("webtransport-h3")`, and `mlmsub` reports `http3: parsing frame failed: Application error 0x0 (remote)`. The relay's library, `web-transport-proto` 0.6.2 under `web-transport-quinn` 0.13.2, accepts `webtransport` alone (`src/connect.rs`: `if protocol != Some("webtransport")`), and sends that token itself as a client. Chromium 156 connects to the relay, so it sends that one too;
  - against the `moqtail` relay 0.15.0, `mlmsub` reports `http3: parsing frame failed: EOF`. What that relay printed was not looked at.
- `scripts/ci/interop.sh` applies it with `git apply`, which fails when it no longer fits the source.

## What was seen of it

- `mlmpub` makes a certificate of its own when it is given none (ECDSA, 14 days) and serves its SHA-256 at `/fingerprint` of `-sideport`. A page connects with that hash and nothing else.
- Its catalog is one Object, {0, 0}, in the namespace `mlm/msf/clear`, also after a restart. It answers a Joining FETCH for it, and sends the same Object on the subscription as well. It never sends a delta update: the type exists, and nothing in its source builds one.
- `mlmsub` does not verify the certificate of what it connects to (`InsecureSkipVerify: true`, no flag).
- `mlmsub`, once it has a catalog, looks for tracks whose names have `_avc` and `_aac` in them. With none, it prints `no matching tracks found`, stays until its `-duration` is over, and exits with 1 (`context deadline exceeded`). It reads a catalog whose `tracks` is empty as well.
- What it writes with `-catalogout` is the catalog written anew, not the bytes it got: an empty `tracks` is left out.
- It always writes a qlog, by default into the directory it runs in; the run points it at `interop/.logs/`.
