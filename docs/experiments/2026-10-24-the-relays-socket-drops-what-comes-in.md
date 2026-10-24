# The relay's socket drops what comes in, and the Objects of the publishers come late to the hub

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, relay scale/load testing

## Objective

The [record before](2026-10-24-why-a-run-of-1000-viewers-with-sound-falls-behind-with-cores-to-spare.md) left the publishers' side of the runs of 1,000 viewers with sound that fall behind. One step more: when each Object leaves its publisher, and when the relay's hub takes it.

## Environment

As in that record: podman's machine on the Mac (Linux 7.1.8, 5 cores, 8 GiB), one container, the 320x180 video and the sound-shaped track (an Object a stream), a viewer two sessions; main at `07ad18da` with the two flags below.

## Commands

```sh
RELAY_FLAGS='--log-arrivals on' PUBLISHER_FLAGS='--log-written on' \
  spikes/fanout/linux/run.sh arr-1000 1000 small.h264 1
python3 spikes/fanout/linux/arrivals.py spikes/fanout/linux/work/logs/arr-1000
```

- `--log-written on` (the Rust client with `--video-in`): a `WRITTEN` line on stderr for each Object, with its Timestamp, when its stream was open, when the transport took its bytes, and the connection's congestion window, round trip and lost packets then.
- `--log-arrivals on` (the relay): an `ARRIVED_STREAM` line for each subgroup stream of a peer and an `ARRIVED` line for each Object, with the time its hub took it, on stderr. Both clocks are the one of the machine.
- `fanout.sh` keeps the relay's and the publishers' stderr apart now, and writes the UDP counters of the container's network (`/proc/net/snmp`) before the viewers and after.

## Observed results

### Where the time goes

| Run | Viewers kept up | The publishers: Timestamp to written (highest) | Timestamp to the relay's hub, video (median / 99th percentile / highest) | The same, sound | The publishers' connections |
|---|---|---|---|---|---|
| 1 | no | 2.6 ms | 799 ms / 9.5 s / 10.2 s | 234 ms / 2.6 s / 2.7 s | not logged |
| 2 | yes: every frame of the video in order, the worst 67 ms | 0.3 ms | 0.4 / 36 / 58 ms | 2 / 36 / 63 ms | 23 and 26 packets lost; the congestion window down to 2,904 bytes |
| 3 | no: the video up to 821 ms | 1.2 ms | 0.3 / 516 / 576 ms | 2 / 140 / 193 ms | 50 and 40 packets lost; the window down to 2,904 bytes |
| 4 | no: the video up to 1.6 s | | up to 5.8 s | up to 791 ms | the kernel dropped 232,648 of 2,650,316 datagrams for a full receive buffer (`RcvbufErrors`) |

- **The publishers write in time**: an Object's bytes are taken by the publisher's transport within 3 ms of its Timestamp, in every run.
- **They come late to the relay's hub**: in the runs that fall behind, the Objects reach the hub seconds after they were written, the video more than the sound. In run 1 the lateness grew by about a second every second for six seconds and then went down at the same pace, as a queue does that is let out at a fixed rate.
- **The publishers' connections lose packets on loopback**: 23 to 50 a run; their congestion windows fall to two packets. In run 4 the kernel dropped 232,648 datagrams of the container's sockets because a receive buffer was full. The relay receives all its sessions on one UDP socket, with Linux's default receive buffer (`net.core.rmem_default`, 212,992 bytes, in this machine; `rmem_max` is 4 MiB).

### A larger receive buffer for the relay's socket

Tried: the relay's socket bound by hand and given 4 MiB (`UdpSocketState::set_recv_buffer_size` of quinn-udp) before quinn takes it. Three runs:

| Run | Datagrams dropped | The publishers' connections | Timestamp to the hub, video (median / 99th percentile / highest) | Viewers, the video's worst 99th percentile | The relay's processor time |
|---|---|---|---|---|---|
| 1 | 0 | no packet lost | (not read: a line of the log was cut) | 3.0 s | 38.9 s |
| 2 | 0 | no packet lost; window 12,000 bytes and more; round trip at most 80 ms | 2.7 s / 15.6 s / 16.6 s | 4.2 s | 44.2 s |
| 3 | 0 | the same | 3.1 s / 16.7 s / 17.6 s | 3.6 s | 43.6 s |

- **Nothing was dropped any more, and the runs fell behind all the same**, all three, the Objects up to 17 s late at the hub, the relay's processor time higher than in the runs without (21.5 s to 34.3 s). The change was not kept: `crates/moqt-io` is as on main.
- So the drops are real and take the publishers' windows down, but they are not what holds the Objects. With no loss, a window that stays open and a round trip of tens of milliseconds, the bytes of the publishers still reach the hub seconds after they were written: they wait in the relay between its QUIC connection of the publisher and its hub. The hub is not behind ([record before](2026-10-24-why-a-run-of-1000-viewers-with-sound-falls-behind-with-cores-to-spare.md)); what is left is the relay's reading of the publishers' streams (the connection's driver of quinn, and the reader of each stream, both tasks among the thousands of the run). Not measured further.

## Kept

- `--log-written on` of the Rust client and `--log-arrivals on` of the relay, off by default: they are what tells where an Object's time goes on its way in, which the next look at the relay's reading needs. Their lines go to stderr; the lines the interop runs wait for are as they were.

## Limitations

- Five runs with the flags, of which one could not be read (a line of a publisher's log was cut into by another), and three with the buffer; whether a run keeps up varies from run to run.
- The datagrams dropped are counted for the container's network as a whole; that the relay's socket is the one is read from the publishers' lost packets, not counted for the socket.

## Decision or next action

None. The next step would be the relay's reading of a publisher's streams under the load of 2,000 sessions.
