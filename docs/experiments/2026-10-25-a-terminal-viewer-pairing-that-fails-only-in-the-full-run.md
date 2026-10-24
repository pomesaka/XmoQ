# A terminal viewer pairing that fails only in the full run

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: none; the interop runs themselves.

## Objective

"own relay: watch: an own page publishes a catalog, video and sound on one connection, and the terminal viewer finds and plays them" failed in two full runs in a row (`mise run interop -- --base-port 49100`, on 2026-10-24, at `8508aaaa` with the change of [the gatherer asking again](2026-10-24-the-gatherer-asks-a-participant-again.md) and after it was put on `9c5f6c9a`), and passed alone. Is it the load of the machine, a race in the pairing's steps, or a fault?

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.14; Chromium 156.0.8078.4 of playwright-core; the own relay (debug build) and `xmoq-watch` (release build) of this tree. During the two failures four working copies ran full interop runs on the machine at once; swap in use was 16 GB of 17 GB at 23:52 and 17 GB of 18 GB at 00:20.

## Commands

- `mise run interop -- --base-port 49100`, twice, with three other full runs on the machine.
- `bun interop/matrix.ts --only "an own page publishes a catalog, video and sound on one connection" --base-port 49300`, three times; once more with `target/release/xmoq-watch` signed anew (`codesign -s - -f`), so that macOS had never run that binary.
- Two Python scripts of sockets on one port (below).
- `mise run interop -- --base-port 49100` with the machine's lock for full runs (`/tmp/xmoq-interop-full.lock`), no other full run at once, and a `xmoq-watch` that was built at 23:52:40 and not run since.

## Observed results

**The two failures** (204 of 205 each): the driver's wait for `CATALOG` (20 s) ran out. The page connected and published its broadcast; the relay accepted no session but the page's (`SESSION_ACCEPTED` once, `session=0`); the viewer printed nothing and did not exit. In both runs it was the first pairing of the run that starts `xmoq-watch`, which had been built anew just before; the pairings after it that start `xmoq-watch` passed.

1. **The viewer ran none of its code for 20 s.** A viewer that runs leaves by itself: a connect that fails ends the first try with an error after 10 s (`CONNECT_TIMEOUT` of `xmoq-moqt-io`), and `--seconds 8` ends it after 8 s. The harness then stops waiting and says `exited with code N before ...`. Neither happened. A race between the page and the viewer, or a fault of the session or of a request, would have reached the connect or one of the two times.
2. **Not a port taken by another socket.** With `--base-port 49100` the pairing's port, 49176, is in macOS's range of ephemeral ports (49152 to 65535). On this machine, a socket on `[::]` without `IPV6_V6ONLY`, as the relay binds, made an IPv4 bind of the same port fail with `EADDRINUSE`, on `127.0.0.1` and on `0.0.0.0`; and of 2,000 IPv4 sockets given any port beside 2,000 such `[::]` sockets, none got a port one of them held.
3. **Not the first run of a new binary alone.** Alone, the pairing passed three times, and once with a binary macOS had never run.
4. **The full run alone passes.** With the lock and no other full run, 205 of 205 pairings ended as expected (430.9 s, from 00:20:09), and this pairing was the first run of a `xmoq-watch` built at 23:52:40 and not run since.
5. **What differed was the load**: the failures came while four full runs went on at once, with the swap nearly full.

## Limitations and blockers

What held the viewer's process for 20 s was not seen: the system log had nothing on it, and no failing run was watched as it happened. The guess is that its start waited on the memory pressure, together with the check macOS makes of a binary on its first run; neither was confirmed. Full runs on this machine are now run one at a time (the lock above).

## Open questions raised

None.
