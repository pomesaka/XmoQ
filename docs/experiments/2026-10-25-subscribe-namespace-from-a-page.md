# SUBSCRIBE_NAMESPACE from a page

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, meetings: finding the participants (undecided; the part both of its choices can use is this one, which the Rust session has since #263).

## Objective

`@xmoq/moqt` decoded NAMESPACE and NAMESPACE_DONE and sent no SUBSCRIBE_NAMESPACE. With `MoqtConnection.subscribeNamespace`, is a page told of a namespace an own-rust publisher announces under its prefix, and of its end, through the own relay and through `moqtail`'s, and does a cancelled request leave the relay free to take another for the same prefix?

## Expected

Written before the first run, at the top of `interop/namespaces.ts`: with `--watcher own-browser`, the page's lines are `NAMESPACES_SUBSCRIBED`, `NAMESPACE namespace=xmoq/phase0` and `NAMESPACE_DONE namespace=xmoq/phase0`, in that order and no others; after it cancels the request the fourth is `NAMESPACES_ENDED cancelled`; a second SUBSCRIBE_NAMESPACE for the same prefix on the same connection is then answered (the relay let go of the first, which it would otherwise refuse the second for, `PREFIX_OVERLAP`, §10.18).

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 of playwright-core, headless; the own relay of this tree (debug build) and `moqtail`'s relay@0.15.0 as `scripts/ci/interop.sh` builds it; `main` at `a75a35cb` with this change for `mise run interop -- --only "namespaces:" --base-port 31500`; the change was then put on `main` at `1dadfd5a`, where the same ran again.

## Result

**As expected, through both relays, in the first run.** All four `namespaces:` pairings ended as expected (8.3 s): the two with own-rust as before, and the two with the page (2.7 s each). The page's lines were the four above, and the second SUBSCRIBE_NAMESPACE was answered through both relays.

On `1dadfd5a` the four ended as expected again (3.3 s). **The whole matrix was not run with this change**: the machine's lock was held for a long measurement, and only the `namespaces:` pairings were run.

## What was not run

- `moqtail`'s relay was not seen to refuse with `PREFIX_OVERLAP`: the second request came after the first was cancelled, as it was meant to. Whether it refuses an overlapping prefix at all was not tried.
- A peer that names more than 4,096 namespaces, a NAMESPACE_DONE before its NAMESPACE, and a reader that does not read `changes` (64 leave the FIN behind them unread, 63 do not; with the bound raised to 1,000 for a run, the row of 64 failed, and the bound was set back): the tests of `@xmoq/moqt` alone (`test/core/session-subscribe-namespace.test.ts`, `test/webtransport/subscribe-namespace.test.ts`).
- A token on the request: the runs have no authorization.
