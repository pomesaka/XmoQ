# The room out of the spikes

- Date: 2026-10-26
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, "Validate Durable Objects integration": the room was `spikes/durable-objects-room`, JavaScript outside every lint, type check and review; it is `@xmoq/room` (`packages/room`), TypeScript, with its decisions in `src/core`, and the form of its token in `fixtures/room-token/`, which `crates/room-token` reads as well. Done on the provisional answer to a question the user has not answered yet.

## Objective

Does the room do what it did, now that it is TypeScript, is built with `bun build` and is run by `node run.ts`?

**Loopback, one machine.**

## Expected

Said before the first run (to the PM, not in a file): nothing that a client sees changes. The checks the spike had (`check.mjs`) pass against the new room, as the new `check.ts` does, and the demo's pages publish, watch and meet through it, with tokens.

## Environment

macOS 14.4, arm64, Node.js 24.21.0, Miniflare 4.20260730.0 (workerd 1.20260730.1), Bun 1.3.14; `node packages/room/run.ts --port 35787`, once with `AUTH_KEY_HEX`, `RELAY_URL`, `RELAY_CERT_SHA256_HEX`, `TRANSCRIPT_PREFIX=xmoq/demo` and `MEETING_TRANSCRIPT_PREFIX=xmoq/meet/{room}`, and once with none. The demo: `bun demo/serve.ts --relay-port 35443 --port 35080 --rooms-port 35787 --auth`, its pages in Chromium of Playwright 1.64.0, headless.

## Result

**As expected.**

- With the settings, `check.ts` and the spike's `check.mjs`, run against the new room, both ended `ALL CHECKS PASSED`: the broadcast while its publisher's socket is open, a token of the form `v1|xmoq/demo|sub|<expiry>|<mac>`, none for a field `*`, a `/` in a field or a meeting's namespace, no chat, the count, the relay and the prefixes in the hello, and a meeting of 49 with the 50th told it is full and let in when one goes. Without the settings, `check.ts` ended so as well, with no token.
- The demo: a publishing and a watching page, with the room's socket of each closed by the test for 3 s, as in [the record of the pages joining their room again](2026-10-26-the-pages-join-their-room-again.md). The watching page played (316 frames in 15 s) after 5 connections. A meeting page was seated and publishing, and seated again after its socket came back.
- The first try failed before any check: workerd would not take a module outside the directory it was started in (`can't use ".." to break out of starting directory`), as `bun build` writes the bundle to a directory of its own under the system's temporary one. `modulesRoot` of Miniflare set to that directory made it start.
- The table of `fixtures/room-token/` (three tokens, made with Python's `hmac`) is read by both: the rows of `crates/room-token` and of `packages/room/test/token.test.ts` pass.

What changed: an `AUTH_KEY_HEX` that is set and is not hexadecimal of one byte at least, the empty one among them, stops `run.ts` before Miniflare starts, and the Worker throws for it, as the relay stops for such an `--auth-key-hex`; one that is not set is no key, and `demo/serve.ts` sets none without `--auth`, where it set an empty one. Before, the spike made a key of what `parseInt` made of a text that is not hexadecimal, and took an empty one for no key.

## Not looked at

- Miniflare under Bun: `run.ts` runs under Node.js, as the spike's did.
- Node.js says "Type Stripping is an experimental feature" as it runs `run.ts`: the demo prints it once.
- A Durable Object of Cloudflare: still Miniflare only.
