# moqlivemock as an interop peer

[`Eyevinn/moqlivemock`](https://github.com/Eyevinn/moqlivemock) v0.16.1 (commit `91d903e9933a452f8baaa860f9fcd20cf195e077`), a publisher and a subscriber for draft 18 of MoQ Transport that are not ours, with an MSF catalog and LOC media. None of its source is kept here: `scripts/ci/interop.sh` builds two commands from the tag into `.tools/`, which is not committed.

| In `.tools/` | What it is here |
|---|---|
| `bin/mlmpub` | The publisher, which is a server: a client connects to it, and no relay is in between. `interop/catalog.ts` has an own page read its catalog |
| `bin/mlmsub` | The subscriber. Built with the publisher and not run by any pairing yet: it is for the run in which it reads a catalog an own page publishes. `-catalogout <file>` makes it write the catalog it read, which that run can compare |
| `assets/test10s` | The media `mlmpub` serves, copied from the tag: 7.6 MB. Its `-asset` flag defaults to a path relative to its source, so the run passes this one |

## Build

`mise run interop` does it, and fails when the tag is no longer the commit above. By hand, from the repository root, with the Go of `mise.toml`:

```sh
git clone --depth 1 --branch v0.16.1 https://github.com/Eyevinn/moqlivemock /tmp/moqlivemock
(cd /tmp/moqlivemock && go build -o "$OLDPWD/interop/peers/moqlivemock/.tools/bin/" ./cmd/mlmpub ./cmd/mlmsub)
mkdir -p interop/peers/moqlivemock/.tools/assets
cp -R /tmp/moqlivemock/assets/test10s interop/peers/moqlivemock/.tools/assets/
```

- **`go install github.com/Eyevinn/moqlivemock/cmd/mlmpub@v0.16.1` does not work.** The `go.mod` of the tag has a `replace` directive (a fork of `webtransport-go`), and `go install` refuses that for a module that is not the main one.
- Measured on macOS, arm64, 11 cores, with empty Go caches: the clone 4.6 seconds and 16 MB, the build of the two commands 9.8 seconds, a second build 0.17. The binaries are 19 MB and 14.6 MB.

## What was seen of it

- `mlmpub` makes a certificate of its own when it is given none (ECDSA, 14 days) and serves its SHA-256 at `/fingerprint` of `-sideport`. A page connects with that hash and nothing else.
- Its catalog is one Object, {0, 0}, in the namespace `mlm/msf/clear`, also after a restart. It answers a Joining FETCH for it, and sends the same Object on the subscription as well. It never sends a delta update: the type exists, and nothing in its source builds one.
- `mlmsub` does not verify the certificate of what it connects to (`InsecureSkipVerify: true`, no flag).
- It always writes a qlog, by default into the directory it runs in; the run points it at `interop/.logs/`.
