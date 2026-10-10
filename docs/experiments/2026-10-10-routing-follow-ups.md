# Routing from a moqtail-ts publisher, and how soon the relay notices a vanished peer

- Date: 2026-10-10
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, route by namespace in the relay; and the runs in which a peer disconnects

## Objective

1. [The routing record](2026-10-10-relay-namespace-routing.md) ended with no Object routed from a publisher written by someone else: moqtail-ts copies the parameters of a SUBSCRIBE into its SUBSCRIBE_OK, the relay's SUBSCRIBE carried FORWARD, and the relay closed the session for a FORWARD in SUBSCRIBE_OK (run 10 there). A SUBSCRIBE without FORWARD means the same Forward State (§10.2.12). Does the run pass when the relay leaves the parameter out, with the check unchanged?
2. The relay took 12.0 seconds to notice a peer that was killed, with an idle timeout of 9 seconds (measured in the runs with disconnects, [record](2026-10-10-disconnects-through-own-relay.md)). Why, and how long does it take with an idle timeout of 6 seconds?

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Own relay and client | `xmoq-relay` and `xmoq-moqt-client` 0.0.0 on `web-transport-quinn` 0.13.2, `quinn-proto` 0.11.19, debug builds |
| moqtail peers | moqtail-ts (`moqtail` npm 0.13.0) in Chromium 156.0.8078.4, headless, through `playwright` 1.64.0; the moqtail Rust client (`moqtail` crate 0.15.0) |

Provisional settings: idle timeout 6 seconds, keep-alive 3 seconds.

## Commands

```sh
mise run check
mise run interop
cargo build --workspace

P="node spikes/moqtail/browser/run.mjs --relay own --publish-namespace"
$P --direction browser-to-rust --rust-subscriber own                 # 1
$P --direction browser-to-rust --rust-subscriber moqtail             # 2
$P --direction browser-to-browser --subscribers 2                    # 3

# port, kills, seconds between PUBLISHED and the kill, arguments for the relay
python3 spikes/relay-idle-timeout/kill_time.py 44581 3 0.5
python3 spikes/relay-idle-timeout/kill_time.py 44582 3 2.7
python3 spikes/relay-idle-timeout/kill_time.py 44583 3 4.5
python3 spikes/relay-idle-timeout/kill_time.py 44584 3 0.5 --idle-timeout-ms 2000
python3 spikes/relay-idle-timeout/kill_time.py 44585 1 0.5 --idle-timeout-ms 9000
```

The nine runs of the routing record (its runs 1 to 9) were repeated as well.

## Observed results

### A moqtail-ts publisher behind the relay

**Passed: in runs 1 to 3 every subscriber received the payload that the moqtail-ts page sent.** The page announces its namespace, registers the track and sends no PUBLISH; the relay's SUBSCRIBE reaches it when the subscriber asks.

| # | Publisher | Subscriber | Result |
|---|---|---|---|
| 1 | moqtail-ts | own Rust client | payload matched |
| 2 | moqtail-ts | moqtail Rust client | payload matched |
| 3 | moqtail-ts | two moqtail-ts pages | payload matched at both |

In run 1 the relay logged `OPEN_REQUEST ... SUBSCRIBE` to the page, then `SUBSCRIBE_OK` to the subscriber, then `OPEN_DATA`. The check that closed the session in run 10 of the routing record is unchanged; what changed is that the relay's SUBSCRIBE has no parameters. This is the first time an Object went through the relay's routing from a publisher that is not own code, and in run 2 and 3 neither end was.

The nine runs of the routing record passed again.

### Noticing a killed peer

The client is killed with SIGKILL some time after its PUBLISH was accepted, and the time until the relay prints `CONNECTION_ENDED ... Lost` is measured.

| Idle timeout | Quiet time before the kill | Seconds until noticed, three kills |
|---|---|---|
| 6 s (default) | 0.5 s | 8.5, 8.5, 8.5 |
| 6 s | 2.7 s | 6.3, 6.3, 6.3 |
| 6 s | 4.5 s | 7.6, 7.6, 7.6 |
| 2 s (`--idle-timeout-ms 2000`) | 0.5 s | 2.5, 2.5, 2.5 |
| 9 s, the old value | 0.5 s | 11.5 (one kill) |

**The time is the idle timeout plus what was left until the relay's next keep-alive.** The idle timer starts again when a packet of the peer arrives, and also when the relay sends its first ack-eliciting packet after one (RFC 9000 §10.1; `permit_idle_reset` in `connection/packet_builder.rs` of quinn-proto 0.11.19). The relay's keep-alive is due 3 seconds after the last packet the relay received or sent. A peer killed 0.5 seconds after its last packet is followed by a keep-alive 2.5 seconds later, and the timer runs from there: 2.5 + 6 = 8.5. Killed 2.7 seconds after: 0.3 + 6 = 6.3. With 4.5 seconds of quiet the keep-alive at 3 seconds was answered, the next one is due at 6: 1.5 + 6 = 7.5, where 7.6 was measured. The 0.1 seconds are not explained; the script times the kill with a sleep and reads the relay's line through a pipe, and no other row is off by as much. With the old 9 seconds: 2.5 + 9 = 11.5, which fits the 12.0 seconds seen before for a peer killed right after its last packet.

So with 6 seconds a vanished peer is noticed after 6 to 9 seconds.

`mise run check` ended with `OK` for all four parts, and `mise run interop` with `45 of 45 pairings ended as expected` (32 before the runs in which a client leaves were added to it; both were run).

## Limitations and blockers

No blocker.

- The moqtail-ts publisher was run with one Object and without an ending: what the relay does with its PUBLISH_DONE, or when it leaves, was not run with this peer.
- A publisher that answers the relay's SUBSCRIBE with `DUPLICATE_SUBSCRIPTION` and then sends PUBLISH is covered by a test of `xmoq-relay-core` only. No peer at hand answers that way: the own clients answer `NOT_SUPPORTED`, and moqtail-ts answers SUBSCRIBE_OK.
- The kill times are from one host on loopback, where a round trip takes no time. Loss detection repeats the unanswered keep-alive; whether a lossy link changes the times was not tried.
- The relay prints the code of a STOP_SENDING it receives on a request stream a peer opened and on a subgroup stream it writes (`STOP_SENDING_RECEIVED ... code=`), not on its control stream nor on a request stream it opened itself. The Rust client prints it for the request stream of its subscription and for a subgroup stream it writes. On a subgroup stream both see a stop only when they next write to the stream. No run here made a peer send one with a code.
- **The code of the resets after a kill was not measured again.** A publisher that vanishes had its open streams reset downstream with 0x0 in about half the cases and 0x1 in the others; the relay now uses 0x1 for both, which a test of `xmoq-relay-core` shows and no run with a real peer does.
- The wait after `DUPLICATE_SUBSCRIPTION` is in vain when the publisher sends the code for a subscription it still holds and not for a PUBLISH (implementation notes, known gaps). How often that happens was not measured.
- These runs are scripts under `spikes/`, not part of `mise run interop` or CI.

## Open questions raised

1. Is a moqtail-ts publisher that only announces its namespace added to the interop matrix, now that it passes?
2. Are 6 seconds without a packet too short for a viewer on a poor link? The value is provisional; a browser sends no keep-alive of its own, and the relay's keep-alive every 3 seconds is what keeps a quiet session up.
