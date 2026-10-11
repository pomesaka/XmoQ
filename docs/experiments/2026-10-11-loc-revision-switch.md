# `@xmoq/loc` follows another revision by one line: a check with a made-up one

- Date: 2026-10-11
- Commit SHA: the commit that puts what is of LOC-04 into `packages/loc/src/draft-04/`, together with this record. The runs were made on its working copy, on `main` at `20452147` with the video packages below it
- Roadmap item: none. The revision of LOC was decided on this day (LOC-04, on transport draft 18), and the package is to be easy to move to another

## Objective

`@xmoq/loc` had the numbers of LOC-04 and the functions that use them in the same files. They are apart now: what is of the revision in a directory with its name, and one import in `src/index.ts` that chooses it. The runs are to show that

1. nothing does anything else than before;
2. another revision is that one line, and what fails then is what has to.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64 |
| Runtime of the tests | Bun 1.3.14 |
| Browser, for the one run of the interop matrix | Chromium 156.0.8078.4, headless |

## Commands

```sh
mise run check
mise run interop -- --base-port 45300
# with the made-up revision in place, each package by itself
(cd packages/loc && bun test)
(cd packages/media && bun test)
(cd interop && bun video.ts --video from-mlmpub)
```

## Observed results

### Nothing does anything else

| | Before | After |
|---|---|---|
| `@xmoq/media`, the names of the tests | 148 names, 157 tests | the same 148 names, 157 tests |
| `@xmoq/media`, assertions | 371,082 | 371,086 (four more: a length checked in each of four rows of `video-sender.test.ts`, where the test reads the time with `readLocTiming` now; counted by the reviewer) |
| `@xmoq/loc` | 46 tests | 61: the 46, 7 for what holds whichever revision it is, and 8 for `beginsWithStartCode`, which came from `@xmoq/media` |

- Three things of `@xmoq/media` were changed with it, and none in what it does: the check for a start code of 4 bytes is `@xmoq/loc`'s now; its tests make the properties they hand in with `locProperties`, where they wrote `0x10` out; and the subscription of its tests logs the time of an Object as it is read, and not the number of a property.
- `mise run check` passed. `mise run interop -- --base-port 45300`: 104 of 105 pairings. The one that failed is the pairing in which `mlmsub` reads a catalog through the own relay, which fails at times on this working copy for a reason of its own ([record](2026-10-11-mlmsub-joining-fetch-race.md)): the relay of the commit below does not have the change that ends it.

### Another revision

A copy of `src/draft-04/` as `src/draft-xx/`, with one number changed, the Timestamp `0x06` where LOC-04 has `0x10` (what LOC-02 has), and another name; and the import of `src/index.ts` pointed at it. **Nothing else was touched.** Both were taken away afterwards and are in no commit.

| | With the made-up revision |
|---|---|
| `@xmoq/loc` | 1 of 61 fails: the test that says which document the package implements. The tests of `test/draft-04/` pass, as they are of LOC-04 and import it by its directory |
| `@xmoq/media` | 2 of 157 fail, and type-checks: the two tests that read the stream of `moqlivemock` in `fixtures/moqt-18/`, whose Objects have the Timestamp of LOC-04 |
| `@xmoq/msf`, `@xmoq/moqt` | none fails |
| The interop run in which a page plays the video of `moqlivemock` | fails: no frame, every Object dropped as having no time |

- **What does not fail is what must not**: what the sender of `@xmoq/media` writes, its receiver reads, in both mappings and out of order; the tables of the receiver, the sender and the publisher; the sequences at random.
- A first try had 20 tests of `@xmoq/media` fail. 14 of them were tests of the publisher that are about streams and failures: the subscription of the tests printed the number of a property into its log. 4 were a test of the sender that wrote `0x10` out as what it expects. Both were changed to read the time with `@xmoq/loc`, and the number is pinned where it belongs, in `packages/loc/test/draft-04/`.

## Limitations and blockers

- The made-up revision differs in one number. A revision that differs in more than numbers (the forms of the payload, what a Timestamp without a Timescale means) was not tried; those are in the directory of the revision as well, and whether `LocRevision` is wide enough for a real second revision is not known before one is written.
- One revision at a time, chosen when the package is built. Nothing speaks two.
- No Rust code reads LOC; the same shape is to be given to it when it does.

## Open questions raised

None.
