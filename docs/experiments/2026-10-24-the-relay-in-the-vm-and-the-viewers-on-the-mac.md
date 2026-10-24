# The relay in the VM and the viewers on the Mac

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, relay scale/load testing; the goal of Q41

## Objective

The [ten rooms](2026-10-24-ten-rooms-of-a-hundred-viewers.md) fell behind with the viewers on the relay's 5 cores. Whether the relay alone keeps up, with the viewers off those cores: the relay and the publishers in the VM, the viewers on the Mac itself, with no second machine to hand.

## Environment

- The relay and the 20 publishers (10 rooms, video and sound as in the ten rooms) in a container of podman's machine (5 cores, 8 GiB), relay of `136f3afd` with `--release`.
- The viewers on the Mac (11 cores, 18 GiB, already short of free memory before the runs: 33% free by `memory_pressure`), the Rust client of the same commit built for macOS with `--release`, two processes a room.
- Between them podman's forwarding of the UDP port (`-p 127.0.0.1:48860:48860/udp`), through its `gvproxy` on the Mac.
- **The clocks differ**: the VM's is 21.1 ms ahead of the Mac's (the midpoint of 200 round trips over TCP of 0.09 to 0.12 ms, the 20 shortest within 0.01 ms). A viewer's delay is its clock less the publisher's Timestamp, cut at 0 (`crates/moqt-client/src/video.rs`), so here it reads 21.1 ms short, and anything under 21.1 ms reads 0.

## Commands

```sh
spikes/fanout/linux/setup.sh
cargo build --release -p xmoq-moqt-client
spikes/fanout/linux/outside.sh out-10x50 10 50    # 10 rooms of 50 viewers
```

`outside.sh` starts `relay-and-publishers.sh` in the container and the viewers on the Mac, and samples the Mac's memory (`vm_stat`). The processor of `gvproxy`, `krunkit` and the viewers was sampled with `ps` once a second, outside the scripts.

## Observed results

| Rooms x viewers | Relay, processor (mean / highest) | Relay, memory | Delay, video: median / worst 99th percentile (as read); over 500 ms | Sound: worst 99th percentile; over 500 ms | QUIC RTT at the relay, highest | The Mac |
|---|---|---|---|---|---|---|
| 1 x 1, nothing else | 2% / 3% | 7 MB | 0 / 0 ms (under 21 ms) | 0 ms | under 1 ms for the viewers' sessions | no swapping |
| 10 x 25 | 45% / 77% | 76 MB | 0 / 0 ms (under 21 ms); 0 | 0 ms; 0 | 18 ms | no swapping; `gvproxy` 16% mean, 73% highest |
| 10 x 50, run 1 | 120% / 178% | 690 MB | 0 / 8,471 ms; 496 of 500 | 3,583 ms; 496 | 62 ms | 33,728 pages swapped out |
| 10 x 50, run 2 | 116% / 180% | 721 MB | 1,210 / 6,599 ms; 491 of 500 | 3,441 ms; 483 | 283 ms | 36,388 pages swapped out; `gvproxy` 60% mean, 206% highest; `krunkit` 131% mean |

- **The path adds nothing measurable with nothing on it**: under 1 ms of RTT, and a delay under the 21 ms the clocks hide.
- 250 viewers kept up. **500 viewers (10 rooms of 50) fell seconds behind in both runs**, where with the viewers in the VM [the same 500 kept up within 10 ms](2026-10-24-ten-rooms-of-a-hundred-viewers.md). No packet was lost, and the VM's cores were not all busy: 2.1 of 5 idle at the mean in run 2.
- So what fell behind here is not the relay's cores but what is between, or the Mac: the RTT went from under 1 ms to 283 ms, `gvproxy` (a user-space forwarder on the Mac) took up to two cores, and the Mac swapped out about 570 MB of pages a run.
- The runs were stopped at 10 rooms of 50, as the Mac was swapping; 10 rooms of 100 was not run.

## Limitations

- The VM's network is not a machine's: every datagram to and from the viewers goes through `gvproxy` on the Mac, and the Mac was short of memory before the first run.
- Two runs of the shape that fell behind, one of each other.
- The delay reads 21.1 ms short (the clocks), and is to arriving, not to being shown.

## Decision or next action

None. This does not tell whether one relay alone keeps 10 rooms of 100 within 500 ms: with the viewers outside the VM the path through `gvproxy` and the Mac fall behind before the relay does. Telling it needs a second machine for the viewers, on a real network to the relay's. Read against the goal as decided later the same day (the rooms may be split over relays): one relay took 500 viewers in all; the goal may be reached with the rooms split over relays, which was not measured.
