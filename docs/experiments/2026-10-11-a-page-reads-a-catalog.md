# A page reads the MSF catalog of a publisher that is not ours

- Date: 2026-10-11
- Commit SHA: the commit that adds `followCatalog` to `@xmoq/msf` together with this record. The runs were made on its working copy, on `main` at `6ad5240c` with the two changes that add `@xmoq/msf` below it
- Roadmap item: Phase 1, "MSF catalog track: published by the browser publisher, read by the viewers". The reading half; it does not close the item

## Objective

MSF-01 §5 has "Subscribers accessing the catalog MUST use SUBSCRIBE with a Joining FETCH (offset = 0)". `@xmoq/msf` could read a catalog as bytes and had nothing that reads the track. Before that part was written, three things were not known of the one publisher with an MSF catalog that can be run here, `mlmpub` of moqlivemock:

1. whether a page can connect to it at all (it is a server, and makes its own certificate);
2. whether it answers a Joining FETCH for its catalog;
3. whether it sends delta updates, which `@xmoq/msf` tells apart and does not apply.

Then the part was written, and the run is to show that a page reads that catalog with it.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, 11 cores, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Publisher | `mlmpub` of `Eyevinn/moqlivemock` v0.16.1 (commit `91d903e9`), built with Go 1.26.0 from a tree with no change of ours |
| Draft | draft-ietf-moq-transport-18; draft-ietf-moq-msf-01 |
| Runtime of the tests | Bun 1.3.14 |

## Commands

```sh
mise run check
mise run interop
(cd interop && bun catalog.ts)          # the one run, alone
```

## Observed results

### Before the part was written: a throwaway page with `@xmoq/moqt` alone

| Question | Seen |
|---|---|
| Does a page connect? | Yes, with the SHA-256 `mlmpub` serves at `/fingerprint` of its side port given to `connect` as `serverCertificateHashes` |
| The SUBSCRIBE to `catalog` in `mlm/msf/clear` | SUBSCRIBE_OK with LARGEST_OBJECT {0, 0}, no track properties |
| A Relative Joining FETCH with Joining Start 0 for that subscription | FETCH_OK with End Location {0, 1}; the fetch stream had Object {0, 0}, 2,715 bytes, no properties, and a FIN. `mlmpub` printed `served catalog via FETCH ... joining=true joiningRequestID=0 start="{group: 0, object: 0}" end="{group: 0, object: 1}"` |
| The subscription | **The same Object {0, 0} came on it as well**, in Subgroup 0, although the filter was Largest Object and the Object is the largest. The source says why: "for backward compatibility with subscribe-only clients" (`internal/pub/pub.go`) |
| Delta updates | **None, by the source**: the catalog type has `deltaUpdate`, and nothing builds one. The catalog is one Object at {0, 0}, also after a restart |

So a reader has to take an Object once that comes by both ways, and cannot count on a Group ID that grows over a restart of the publisher.

### The run in the matrix

`interop/catalog.ts`: the own page connects to `mlmpub`, calls `followCatalog` for `mlm/msf/clear`, and reports every state it gets until a second after the first. What has to be seen was written at the top of the driver before the run.

| Expected | Seen |
|---|---|
| One state, and no second one within a second | one |
| Its catalog has the 13 tracks of `fixtures/msf-01/catalog-moqlivemock.json`, field for field as `@xmoq/msf` reads them, at {0, 0}, not outdated | the same, but for `generatedAt`, which is the time `mlmpub` started |
| The page says its Joining FETCH was served, with one Object on the fetch stream | `served 1` |
| `mlmpub` says it served the catalog with a Joining FETCH | `served catalog via FETCH ... joining=true`. The line has no more: that the FETCH is a Relative one with Joining Start 0 is in the tests of `@xmoq/msf`, and cannot be told here, where the catalog is in Group 0 |

- The run takes 2.3 seconds alone.
- Temporary changes to `@xmoq/msf`, each taken back: with the Object that is held taken a second time, the run fails with "2 states of the catalog, not 1"; with no FETCH sent, it fails with "not-sent"; with the Objects of the fetch stream dropped, it fails with "served 0". In the last two the catalog still came, by the subscription.
- **The page's side of the FETCH was added after a review**: at first only the line of `mlmpub` was looked at, which it prints when it has written the fetch stream. A page that could not read that stream would have passed, as the same Object comes on the subscription.
- **A fault of the page of the run, found by the first of those changes**: it read the next state twice at a time, so a second state was lost, and the run passed with the change in place. Fixed before this record.

### In the tests, without a browser

- `catalogStateAfter`, 14 rows: which catalog a reader has after each Object, for Objects that come twice, late, out of order and out of place.
- `followCatalog` against a connection made by hand, with real bytes of a fetch stream: the two requests it sends, the Object that comes by both ways, no FETCH without a LARGEST_OBJECT, a FETCH that is refused or gets no answer, a slow reader, the ends, the signal.
- 23 temporary changes to the implementation, one at a time, each taken back: each makes between 1 and 3 tests fail.

### Building the peer

| | macOS, arm64, 11 cores, empty Go caches |
|---|---|
| A shallow clone of the tag | 4.6 seconds, 16 MB |
| `go build` of `mlmpub` and `mlmsub` | 9.8 seconds (29 seconds of CPU time), with the modules fetched |
| The same again | 0.17 seconds |
| The binaries, and the asset `mlmpub` reads | 19 MB, 14.6 MB, 7.6 MB |

`go install ...@v0.16.1` does not work: the `go.mod` has a `replace` directive.

## Limitations and blockers

- Chromium only, loopback only, one publisher, whose catalog never changes: an update of a catalog, a delta update, a later Group and an Object that cannot be read were run against the session of the tests alone.
- No relay is in the run. A catalog read through the own relay needs a publisher of ours, which is the next change.
- **The time the Go toolchain and the build add in CI was not measured**: this record was written before the first run there.
- `mlmsub` was built and not run.
- **No run has a track with a `namespace` of its own**: no track of `mlmpub`'s catalog has one. The run turns one string into a namespace, the `"mlm/msf/clear"` the driver passes; `trackNamespaceOf` was checked by its tests and by reading `moqlivemock`'s source.

## Open questions raised

- How the string of `namespace` in a catalog becomes the tuple of MoQ Transport is not in MSF-01. `@xmoq/msf` does what `moqlivemock` does (split at "/"); a namespace with a "/" inside a field cannot be named.
