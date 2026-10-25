# One video track through two edges

- Date: 2026-10-25
- Commit SHA: the commit that adds this record (the relay at `main` `1dadfd5a`, with the first three steps of the edge, #328, #332 and #334; the fourth, its tokens, was not in it, and the relays ran without `--auth-key-hex`)
- Roadmap item: Phase 7, relay scale (decided on 2026-10-25: one relay serves 500 viewers of a broadcast, and relays are added to build a broadcast of 10,000); choice B of the [design draft](../scale-and-meetings.md), measured once

## Objective

With an origin and two edges (`--upstream`), the origin is to send each Group once to each edge whatever the number of viewers, and each edge to hold one session with the origin. Measured: the sessions at the origin and of each edge upstream, how many copies of each Group the origin and the edges send, what each relay takes of the machine, and how late the frames come; beside it, the same number of viewers at the origin alone, and whether one edge takes the 500 a relay is to serve (decided on 2026-10-25).

**Loopback, one machine**: the three relays, the publisher and every viewer share one Mac, so the numbers say what the shape costs on that machine, not what two edges on their own machines would take.

## Environment

Apple M3 Pro, 11 cores, 18 GiB, macOS 14.4; the own relay and the own Rust client built with `--release`; ports 34500 (origin), 34501 and 34502 (edges). The video: ffmpeg 8.1 `testsrc2`, 1280x720, 25 frames a second, a key frame a second, `h264_videotoolbox` Baseline at 2,500 kbit/s, 100 frames (the `large.h264` of `spikes/fanout/linux/setup.sh`, made the same way). The publisher is the own Rust client with `--video-in`: a Group a second, a frame every 40 ms, each with the wall-clock time as its LOC Timestamp, at the origin. The viewers are sessions of one Rust client process at each edge (`--viewers`, 10 ms apart), 250 frames each; their lateness is the time a frame came less its Timestamp (`VIDEO_STATS late_us_*`). The full interop's lock was held for the whole of the runs, so that no other run shared the machine.

```
spikes/edge-fanout/run.sh <viewers at each edge> large.h264 250            # two edges
EDGES=0 spikes/edge-fanout/run.sh <viewers at each edge> large.h264 250    # 2N at the origin alone
```

## Result

One run each, all within the lock (10:43 to 10:46; the run of one edge of 500, `EDGES=1`, at 10:55, in the lock as well). Every viewer of every run got every frame (`viewers_with_every_frame` = all), and none failed.

| Viewers | Shape | Sessions at the origin | Copies of a Group: origin / each edge | CPU, mean / highest: origin | each edge | Memory, highest: origin / each edge | Late, ms: median of p50 / worst p99 / max |
|---|---|---|---|---|---|---|---|
| 20 | 2 edges of 10 | 3 | 2 / 10 | 1% / 2% | 5% / 10% | 11 / 16 MB | 1.7 / 6.4 / 10.6 |
| 20 | origin alone | 21 | 20 / – | 26% / 43% | – | 21 MB | 2.5 / 11.4 / 14.2 |
| 100 | 2 edges of 50 | 3 | 2 / 50 | 1% / 2% | 32% / 44% | 11 / 30 MB | 5.2 / 22.4 / 26.9 |
| 100 | origin alone | 101 | 100 / – | 71% / 102% | – | 47 MB | 6.5 / 28.2 / 31.6 |
| 200 | 2 edges of 100 | 3 | 2 / 100 | 1% / 1% | 49% / 74% | 11 / 51 MB | 7.5 / 30.6 / 31.8 |
| 200 | origin alone | 201 | 200 / – | 115% / 166% | – | 97 MB | 9.4 / 55.4 / 75.1 |
| 500 | 2 edges of 250 | 3 | 2 / 250 | 1% / 1% | 107% / 148% | 11 / 109 MB | 14.1 / 74.0 / 111.1 |
| 500 | origin alone | 501 | 382 (median) / – | 185% / 335% | – | 204 MB | 17.0 / 229.1 / 268.1 |
| 500 | 1 edge of 500 | 2 | 1 / 380.5 (median) | 1% / 1% | 115% / 218% | 11 / 192 MB | 12.4 / 119.9 / 158.9 |

- **Each edge held one session with the origin** (`UPSTREAM_CONNECTED` once in each log), and the origin had three sessions, the publisher and the two edges, at every size. It opened **two subgroup streams for each Group, one to each edge**, at every size: the median of the streams it opened a Group. What the origin took stayed at 1 to 2% of a core and 11 MB.
- **One edge took 500 viewers within the goal of 500 ms**: every frame to all of them, at worst 120 ms at the 99th percentile, against 229 ms with the 500 at the origin alone; it took 115% of a core at the mean and 218% at the highest, 192 MB. As at the origin alone, the median of the streams a Group (380.5) is under 500, for the viewers that come 10 ms apart.
- Each edge took about half of what the origin alone took for twice its viewers: 107% at the mean for 250, against 185% for 500 at the origin alone. The two edges together (215%) took a little more than the origin alone, on the same machine.
- The frames came later at the origin alone in each pair, most at 500: 229 ms against 74 ms at the 99th percentile of the worst viewer.
- With 500 at the origin alone, the median of the streams a Group was 382, not 500: the viewers come 10 ms apart, so the first Groups of the run had fewer subscribers.

## Not looked at

- Two machines, or three: an edge on a machine of its own, and real links. Here every relay and viewer shared the machine, so a relay's CPU is also what the others left it.
- More than 500 at an edge, and more than two edges.
- A page as the publisher, and sound: the publisher is the own Rust client, as in the records of [2026-10-11](2026-10-11-one-video-track-to-many-viewers.md) and [2026-10-24](2026-10-24-one-video-track-to-many-viewers-on-linux.md), so that the numbers can be laid beside theirs.
- One run of each size; nothing of how much the numbers vary.
