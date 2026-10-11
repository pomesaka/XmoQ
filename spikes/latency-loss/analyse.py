#!/usr/bin/env python3
"""Reads the VIDEO_ARRIVALS lines of the viewers and says how late the frames were.

    analyse.py <viewers.log> <delay> <loss> <video> <streams>

A frame can be shown when it and every frame of its Group before it have come: that moment,
counted from the frame's Timestamp, is how late it is here. The Objects of a Group a viewer came
into the middle of are left out. The frames are 40 ms apart.
"""
import statistics
import sys

FRAME_US = 40_000
log, delay, loss, video, streams = sys.argv[1:6]
congestion, relay_log, publisher_log = (sys.argv[6:9] + ["cubic", "", ""])[:3]
# The median lateness of each quarter of a viewer's run, over the viewers: whether it grows.
quarters = [[], [], [], []]
late = []          # of every frame of every viewer, in the order of the frames
episodes = []      # lengths, in frames, of runs of frames more than a frame later than the median
never = 0          # frames that could not be shown: one before them did not come
viewers = 0
for line in open(log):
    if not line.startswith("VIDEO_ARRIVALS "):
        continue
    viewers += 1
    groups = {}
    for record in line.split()[1].split(","):
        group, obj, us = (int(n) for n in record.split(":"))
        groups.setdefault(group, {})[obj] = us
    of_viewer = []
    for group in sorted(groups):
        objects = groups[group]
        if 0 not in objects:
            continue
        ready = None
        for obj in range(max(objects) + 1):
            if obj not in objects:
                never += max(objects) - obj
                break
            # When it came, counted from the Timestamp of Object 0 of its Group.
            came = objects[obj] + obj * FRAME_US
            ready = came if ready is None else max(ready, came)
            of_viewer.append(ready - obj * FRAME_US)
    if not of_viewer:
        continue
    median = statistics.median(of_viewer)
    run = 0
    for us in of_viewer + [0]:
        if us > median + FRAME_US:
            run += 1
        elif run:
            episodes.append(run)
            run = 0
    late.extend(of_viewer)
    for quarter in range(4):
        part = of_viewer[len(of_viewer) * quarter // 4 : len(of_viewer) * (quarter + 1) // 4]
        if part:
            quarters[quarter].append(statistics.median(part))
if not late:
    print(f"LATENCY delay_ms={delay} loss={loss} video={video} streams={streams} viewers={viewers} frames=0")
    sys.exit(0)
late.sort()
at = lambda share: late[min(len(late) - 1, int(len(late) * share))] / 1000
fastest = late[0]
within = lambda ms: 100 * sum(1 for us in late if us <= fastest + ms * 1000) / len(late)
import re

cwnd = sorted(int(n) for n in re.findall(r"cwnd=(\d+)", open(relay_log).read())) if relay_log else []
sent = re.findall(r"^SENT .*payload_len=(\d+)", open(publisher_log).read(), re.M) if publisher_log else []
kbit = sum(int(n) for n in sent) * 8 // 1000 // max(len(sent), 1)
grows = "/".join(f"{statistics.median(q) / 1000:.0f}" if q else "-" for q in quarters)
print(
    f"LATENCY congestion={congestion} kbit_s={kbit} delay_ms={delay} loss={loss} video={video} streams={streams} viewers={viewers} frames={len(late)} "
    f"late_ms_by_quarter={grows} cwnd_median={cwnd[len(cwnd) // 2] if cwnd else 0} cwnd_min={cwnd[0] if cwnd else 0} "
    f"never={never} late_ms_p50={at(0.5):.0f} p95={at(0.95):.0f} p99={at(0.99):.0f} max={late[-1] / 1000:.0f} "
    f"episodes={len(episodes)} episode_frames_median={statistics.median(episodes) if episodes else 0:.0f} "
    f"episode_frames_max={max(episodes, default=0)} "
    + " ".join(f"in_{ms}ms={within(ms):.1f}%" for ms in (50, 100, 150, 200, 300))
)
