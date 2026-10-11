# Cubic and BBR over a link with too little room

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay (latency); open question on the relay's default congestion control

## Objective

[The record of the night before](2026-10-12-video-over-a-link-that-delays-and-loses.md) lost packets at random, which is the case BBR is made for. Here the link has a rate and a queue, and loses only what does not fit in the queue: what Cubic and BBR of the relay do with a video that fits the link, barely fits, or does not.

## Environment

One machine, macOS, loopback. `xmoq-relay` and `xmoq-moqt-client` of the working copy this record is part of, built with `--release`. Between the relay and the viewers, `spikes/lossy-link/proxy.mjs`, which now has `rate` (kbit/s of UDP payload, each way on its own, one queue for all the viewers) and `queue` (milliseconds of datagrams that may wait; one that would wait longer is dropped). Random loss 0, 50 ms each way. The video is `fan-720-2500k.h264` of [the fan-out record](2026-10-11-one-video-track-to-many-viewers.md): 1280x720, 25 frames a second, a key frame a second, 2,542 kbit/s of payload, a Group on one stream.

## Commands

```sh
CONGESTION=cubic|bbr RATE=<kbit/s> QUEUE=<ms> spikes/latency-loss/run.sh 50 0 fan-720-2500k.h264 group 750 <viewers>
```

A viewer takes 750 frames (30 seconds). How late a frame is, is counted as in the record before: from its Timestamp to the moment it and every frame of its Group before it have come; the 50 ms of the path are in it. "Overflowed" is the number of datagrams the queue dropped in the run, both ways together.

## Observed results

Ranges over the runs. Nothing but the queue dropped a datagram. The publisher sends at the pace of the file whatever happens, and nobody leaves frames out, so a link that cannot carry the video makes it later and later with either control; **all the frames came** in every run but one (BBR, 2 viewers, 4 Mbit/s, second run: 1,416 of 1,470 before the viewers' time was over).

### One viewer, queue 100 ms, 3 runs each

| Link | Control | Median late, ms | Latest frame, ms | Overflowed |
|---|---|---|---|---|
| 3 Mbit/s | Cubic | 84 | 151–155 | 0 |
| 3 Mbit/s | BBR | 84 | 315–528 | 1–23 |
| 2.5 Mbit/s | Cubic | 592–622 | 1,981–2,005 | 21–25 |
| 2.5 Mbit/s | BBR | 1,777–1,949 | 6,207–7,606 | 11,205–13,104 |
| 2 Mbit/s | Cubic | 3,980–4,818 | 16,267–18,153 | 21–57 |
| 2 Mbit/s | BBR | 4,617–7,525 | 16,840–17,996 | 8,119–27,937 |

### One viewer, queue 25 ms, 2 runs each

| Link | Control | Median late, ms | Latest frame, ms | Overflowed |
|---|---|---|---|---|
| 3 Mbit/s | Cubic | 190–207 | 458–576 | 75–76 |
| 3 Mbit/s | BBR | 289–304 | 740–762 | 3,158–3,377 |
| 2.5 Mbit/s | Cubic | 1,937–2,050 | 6,704–6,818 | 18 |
| 2.5 Mbit/s | BBR | 1,231–4,411 | 4,692–12,166 | 11,146–40,794 |
| 2 Mbit/s | Cubic | 5,392–5,401 | 21,102–21,160 | 25–27 |
| 2 Mbit/s | BBR | 4,820–6,943 | 17,079–17,300 | 11,257–34,392 |

### Two viewers on one link, queue 100 ms, 2 runs each

| Link | Control | Median late, ms | Latest frame, ms | Overflowed |
|---|---|---|---|---|
| 6 Mbit/s | Cubic | 83–97 | 151–2,718 | 0–15 |
| 6 Mbit/s | BBR | 83 | 182–184 | 0 |
| 5 Mbit/s | Cubic | 396–1,210 | 3,481–4,880 | 34–42 |
| 5 Mbit/s | BBR | 1,720–1,748 | 6,364–7,220 | 30,492–31,239 |
| 4 Mbit/s | Cubic | 3,729–3,944 | 17,814–19,466 | 96–104 |
| 4 Mbit/s | BBR | 5,032–9,558 | 18,476–29,288 | 36,415–100,184 |

In one Cubic run at 6 Mbit/s the first quarter of the frames was a second late (15 datagrams overflowed as the two viewers began) and the rest was not.

## Conclusion

- **With room to spare (3 Mbit/s for 2.5, queue 100 ms) both carry the video at the delay of the path.** BBR overflows the queue now and then and has single frames up to half a second late; Cubic had none.
- **Where the link is as wide as the video or narrower, BBR is not better and mostly worse.** It keeps sending more than the link takes: the queue dropped 8,000 to 100,000 of its datagrams in a run, against Cubic's 18 to 104, and every one of them is sent again. At 2.5 Mbit/s the median frame was 1.8–1.9 s late with BBR and 0.6 s with Cubic.
- **A short queue (25 ms) costs both, BBR more**: at 3 Mbit/s, 190–207 ms with Cubic and 289–304 ms with BBR, which lost 3,000 datagrams to it.
- Together with the record before: BBR is the better one where packets are lost for no reason of the sender's, Cubic where they are lost because the link is full. Neither run says which of the two a viewer's path is.
- When the link cannot carry the video, neither control helps: the video falls seconds behind and stays there. That is for the publisher or the relay to answer by sending less (leaving Groups out), which nothing here does.

## Not measured

- Sound next to the video on the same connection: the Rust client publishes one track a run. The video alone was sent.
- A link shared with traffic that is not ours (another TCP or QUIC flow), and a rate that changes while the video runs.
- A real network; the link is a Node process with timers of a millisecond, on the machine that also runs the relay and the clients.
- The other settings of quinn's controllers (`initial_window` and the like), and NewReno.

## Next action

The relay's default stays Cubic; the choice is an open question. A measurement of a path with both kinds of loss at once would say more than either record alone.
