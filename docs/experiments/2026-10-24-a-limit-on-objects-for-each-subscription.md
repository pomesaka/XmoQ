# A limit on the payload of Objects for each subscription

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: none; a known gap of `@xmoq/msf` (a catalog Object on the subscription was collected up to the connection's 16 MiB)

## Objective

`@xmoq/moqt` takes `maxObjectPayloadLength` on `subscribe()`, a limit below the connection's for one subscription, and `followCatalog` of `@xmoq/msf` gives its subscription the catalog's 1 MiB. Check that the catalog pairings of the interop matrix, which run `followCatalog` against `moqlivemock` and through both relays, still pass with the limit set.

## Expected

Written before running.

- `bun test` in `packages/moqt`: an Object at the subscription's limit is delivered; one byte over stops its stream with INTERNAL_ERROR, the reason naming the subscription's limit, while the session, the subscription and another subscription go on; without the option, the connection's limit holds.
- The pairings `--only "catalog:"` pass as before: the catalogs in them are a few hundred bytes, far below 1 MiB, so the limit changes nothing they see.
- The pairings `--only "chat:"` pass as before: nothing they run is changed (`@xmoq/chat` is not in this tree, see Not done).

## Environment

macOS (Darwin 23.4.0), Bun 1.3.14, the Chromium of the interop harness, the workspace on top of `09c82e3e`.

## Commands

```sh
mise run check
bun interop/matrix.ts --only "catalog:" --base-port 45300   # twice
bun interop/matrix.ts --only "chat:" --base-port 45300      # twice
```

## Result

As expected.

| Run | Result |
|---|---|
| `mise run check` | passed; `packages/moqt` 409 tests, among them the 11 of `test/core/session-subscription-limit.test.ts` |
| `--only "catalog:"`, first | 4 of 4 pairings as expected, 6.4 s |
| `--only "catalog:"`, second | 4 of 4 pairings as expected, 5.6 s |
| `--only "chat:"`, first | 2 of 2 pairings as expected, 1.4 s |
| `--only "chat:"`, second | 2 of 2 pairings as expected, 1.2 s |

In the catalog pairings every catalog was read: by the Joining FETCH where it was served, and from the subscription for the replacements, with `followCatalog` giving its subscription 1 MiB.

## Not done

- No run sends a catalog Object over 1 MiB on the subscription through a relay; that path is covered by the unit tests of `@xmoq/moqt` alone.
- `@xmoq/chat` (`packages/chat/src/moqt/participant.ts`) is not in this tree (it is in PR #275), so its `lines` subscription did not get the 64 KiB limit. The chat pairings run the Rust gatherer and viewer, which nothing here changed.
- The Rust implementation has no limit for one subscription (`docs/moqt-implementation-notes.md`, known gaps); nothing was changed there.

After that, `mise run check` passed again and `mise run interop -- --base-port 45300` ended 194 of 194 as expected (383 s).
