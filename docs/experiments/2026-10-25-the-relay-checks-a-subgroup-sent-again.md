# The relay checks a Subgroup sent again

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the relay's handling of a malformed track.

## Objective

The relay stopped a second stream of a source for a Subgroup it had delivered whole without reading it, so the second and third examples of a malformed track in §2.4.2 ("An Object is received whose Object ID is larger than the final Object in the Subgroup", "A Subgroup is received over multiple transport streams terminated by FIN with different final Objects") were never found. It now reads such a stream for the check alone (decided on 2026-10-25: the relay alone, the same source alone, 4 streams a track for 10 s at most). Does a publisher that sends a Subgroup again with one Object more, or one fewer, end its track as malformed for the subscribers?

## Expected

Written before the first run, at `subgroup-again` in `interop/disconnect.ts`:

- `--again longer`: the own-rust publisher sends a Group, then the next Group on a stream with Object 0 that ends with a FIN, then the same Subgroup on a second stream with Objects 0 and 1. The relay reads the second stream; Object 1 is past the final Object, and the track ends as malformed: PUBLISH_DONE `MALFORMED_TRACK` (0x12) to the own-rust and own-browser subscribers, and the PUBLISH cancelled with 0x12.
- `--again shorter`: Objects 0 and 1, then Object 0 alone on the second stream, which ends with a FIN after another final Object: the same.
- Not run against a `moqtail` peer: `moqtail`'s clients have no way to send a Subgroup again, so the publisher is the own Rust client alone (an exception to running an own part against `moqtail` first). `disconnect.ts` drives the own relay alone, so what `moqtail`'s relay does with these streams was not seen either.

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.14; Chromium 156.0.8078.4 of playwright-core, headless; the own relay and the own-rust client of this tree (debug builds), on `main` at `19f24911` with this change.

## Commands

- `cargo test -p xmoq-relay-core` (the tests in `src/tests/subgroups.rs`, with both examples, a first stream without an Object, a second stream without one, a status as the final Object, the limit of 4 and the 10 s).
- `bun interop/matrix.ts --only "sends a Subgroup it delivered whole again" --base-port 49150`, six times.
- `mise run interop -- --base-port 49100` with the machine's lock for full runs, twice, and `mise run interop -- --base-port 39100` once.

## Observed results

**As expected, in every run of the two new pairings** (six alone, and in both full runs): the relay ended the track as malformed when the publisher sent Group 1 again, PUBLISH_DONE 0x12 reached both subscribers, and the relay cancelled the PUBLISH with 0x12, which the publisher saw. The two pairings took 1.4 to 3.2 s each.

**The first full run did not finish**: at its 224th line the driver failed with `ENOSPC: no space left on device, write`; the machine's disk had 120 MB left. Two pairings before it had failed with a client that exited during its start (`pub:own-rust exited with code 1 before printing /^PUBLISHED /`, and `mlmsub exited with code 1`). The new pairings had passed by then.

**The second full run, with 25 GiB free, ended 209 of 210 as expected** (482.3 s, from 01:28:43). The moqlivemock pairing that failed in the first run passed. The one that failed was "own relay: fetch: own-browser fetches the latest Group of own-rust, which answers no FETCH; the relay answers", as in the first run: the relay printed `LISTENING ... port=49155`, and the publisher left with `no session with 127.0.0.1:49155 within 10s ... the QUIC handshake did not complete`. It does not reach anything this change touches. Run alone twice with `--base-port 39100`, it ended as expected both times.

**A third full run, after this change was put on `main` at `15feaa08` (TRACK_STATUS at the relay), with `--base-port 39100`, below the range of ephemeral ports, ended 210 of 210 as expected** (427.5 s, from 02:00:33).

Why it fails at that port: 49155 is in macOS's range of ephemeral ports (49152 to 65535), and on this machine a socket of another process on `0.0.0.0` that holds a port does not keep the relay from binding it. With an IPv4 socket bound to `0.0.0.0:49997` first, a socket on `[::]` without `IPV6_V6ONLY`, as the relay binds, was bound to the same port as well, and a datagram sent to `127.0.0.1:49997` reached the IPv4 socket and not the `[::]` one; with the IPv4 socket on `127.0.0.1` the second bind failed with `EADDRINUSE`. So a relay of a pairing whose port another process holds as its ephemeral port prints `LISTENING` and is not reached over IPv4. When the run failed, 49155 was free again, and which process held it was not seen.

## The second stream after the first was delivered whole (added on 2026-10-25)

In CI on #339 (a change that touches no data path), "own relay: leaving: an own-rust publisher sends a Subgroup it delivered whole again with one Object fewer; the track ends as malformed" failed. The relay's log had the two streams of Group 1 the other way round: the short one (Object 0 alone) reached the relay first, then the long one (0 and 1). The relay reset its stream towards the subscriber with 0x1, stopped the publisher's with 0x1, and passed on the long one to its end. So the Subgroup was not delivered whole before it was sent again. The own-rust client opened the second stream once it had written the first stream's FIN, and QUIC keeps no order between two streams.

**The fix, in the run and in the client of the run alone; the relay is unchanged.** The own-rust client sends the first stream at `subgroup-again-longer` or `subgroup-again-shorter`, and the second at a further line, `subgroup-again-second` (`send_subgroup_twice` of `crates/moqt-client/src/main.rs`). `subgroupAgain` of `interop/disconnect.ts` writes that line once the relay has printed `FINISH_DATA` for each of its streams of the Group towards the subscribers (`OPEN_DATA ... group=1`; `deliveredWhole`).

`mise run interop -- --only "Subgroup it delivered whole again" --base-port 36000`, two pairings a run (the longer and the shorter), on macOS 14.4 on an M3 Pro, with debug builds:

| | Runs | Pairings as expected |
|---|---|---|
| Before the fix, on `main` at `1b8a2f7e` | 10 | 20 of 20 |
| After it | 10 | 20 of 20 |

The failure of CI did not come here before the fix, in 10 runs. What the fix changes was seen in the relay's log of a run after it: `FINISH_DATA out=2` and `out=3` (the relay's streams of the first stream towards the two subscribers) came before the publisher's second `SENT group=1`, and then the relay stopped the publisher's second stream with 0x12 (malformed). Whether CI's failure is gone can be seen only in CI's runs.

## Limitations and blockers

- A second stream whose header the relay reads before the first stream's FIN takes the place of the first, as before, and nothing is compared. The runs do not show this case. Since the section below, the run sends the second stream only once the relay has finished its streams of the first towards every subscriber.
- `--base-port 49100` puts every pairing from the 53rd on in macOS's range of ephemeral ports.

## Open questions raised

- Whether the interop runs keep their ports below macOS's range of ephemeral ports (the default `--base-port` 24400 does), and whether `matrix.ts` refuses a base port whose range reaches into it, as `scripts/ci/interop.sh` prints Linux's range.
