# What the sound waits behind on a narrow link

- Date: 2026-10-15
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link with less room than the video

## Objective

Behind a link of 2 Mbit/s with a video of 2.65 and no delivery timeout on the video, the sound came later and fewer of its pieces came from about 55 s on, with or without a priority above the video and with ten pieces a stream as with one ([record](2026-10-14-three-minutes-behind-a-narrow-link-and-the-relays-limit-for-one-viewer.md)). Read from the times: the relay's send window for a connection (4 MiB, `SEND_WINDOW` in `crates/relay/src/main.rs`) is full then, and a writer that waits for room in it waits whatever its priority. Here that is looked at, and a smaller window is tried.

The relay now writes, with `--stats-every-ms N` (for measuring; off when left out), a line for each session every N ms: `STATS session=… cwnd=… rtt_ms=… lost_packets=… congestion_events=… sent_bytes=… handed_bytes=… writing=… waiting_bytes=… waiting_objects=… streams_not_done=… opening=…`. `sent_bytes` is what the QUIC connection has sent, all packets with their headers; `handed_bytes` what the transport has taken from the subgroup streams; `writing` the writes that wait for the transport now; `waiting_*` what the relay itself holds for the session; `opening` the streams that wait for the peer to allow them. `handed_bytes - sent_bytes` is about what the transport holds and has not sent (less the headers and what was sent again).

## Expected, written before the runs

- **H** (as before, 4 MiB): what the transport holds grows by about 80 KB a second to about 4 MiB at about 50 s, when the sound begins to decline. From then on `writing` is above 0; before, mostly 0. `cwnd` stays at tens of KB: the backlog is in the send buffer, not in flight.
- **I** (the window at 1 MiB, a change made for this run and taken back after): the decline begins earlier, at about 15 s, since a window does not order who goes in.

## Environment

As in the record above: macOS 14.4, arm64, loopback; `spikes/lossy-link/proxy.mjs` between viewer and relay at 2,000 kbit/s, queue 100 ms, 50 ms each way; `/publish?picture=busy&bitrate=2500000`, a piece of sound a stream; `/watch?via=…&buffer=150&videoTimeout=0`; the own relay with no flag but `--stats-every-ms 1000`; 2 minutes each, one run each.

## Result

The viewer's session, every 5 s (the seconds count the relay's lines, from about 3 s before the viewer came):

| s | H: held by the transport, KB | H: writes waiting | H: held by the relay, KB | I (1 MiB): held by the transport, KB | I: writes waiting | I: held by the relay, KB |
|---|---|---|---|---|---|---|
| 10 | 578 | 0 | 0 | 574 | 0 | 0 |
| 15 | 924 | 0 | 0 | 780 | 8 | 153 |
| 20 | 1,275 | 0 | 0 | 737 | 13 | 552 |
| 30 | 2,020 | 0 | 0 | 680 | 25 | 1,342 |
| 45 | 3,066 | 0 | 0 | 546 | 35 | 2,524 |
| 50 | 3,435 | 0 | 0 | 559 | 50 | 2,880 |
| 55 | 3,589 | 5 | 195 | 447 | 52 | 3,344 |
| 60 | 3,520 | 11 | 617 | 425 | 55 | 3,725 |
| 80 | 3,402 | 32 | 2,179 | ended at about 67 s | | |
| 100 | 3,212 | 44 | 3,782 | | | |

- **H, as expected**: what the transport held grew by about 75 KB a second, stood at about 3.4 to 3.6 MiB from 52 s on, and from that moment writes waited for it (0 before, 5 to 50 after) and what the relay held itself began to grow. `cwnd` was 37 to 53 KB throughout. The sound began to decline at 48 to 58 s (pieces a second, read by the page every 5 s: 50 up to 48 s, then 37 at 58 s), and the video's subscription was ended at about 105 s, when the relay's own 4 MiB were full.
- **I, as expected**: the transport held at most about 780 KB, writes waited from 15 s on, the sound declined from about 20 s (26 pieces a second at 28 s) and the subscription was ended at about 67 s. **A smaller window makes it come sooner**, and makes nothing better.
- So what the sound waits behind is the room in the transport's send buffer, which every writer of the connection waits for alike. A priority orders what is in the buffer, and the sound's bytes do not get into it.
- Not run: a clean link with the smaller window (whether a fast viewer is slowed), since the smaller window was no help.
