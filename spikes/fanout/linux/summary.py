"""The FANOUT lines of spikes/fanout/run.sh, for the video's viewers and the sound's, and a
summary of the relay's STATS lines and resets."""
import collections, glob, os, re, statistics, sys

logs, viewers, video, frames, pieces = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4]), int(sys.argv[5])
cpu, rss, video_cpu, sound_cpu, busy = [], [], [], [], []
# time, relay ticks, relay RSS, video viewers' ticks, sound viewers' ticks, machine idle ticks
samples = [tuple(map(float, line.split())) for line in open(f"{logs}/relay.ticks") if len(line.split()) == 6]
CORES = os.cpu_count() or 1
# The seconds while the viewers of the video ran: after they end their ticks read 0.
for a, b in zip(samples, samples[1:]):
    if a[3] <= 0 or b[3] <= 0:
        continue
    span = b[0] - a[0]
    cpu.append((b[1] - a[1]) / span)  # ticks of 1/100 s a second: percent of a core
    rss.append(int(b[2]))
    video_cpu.append((b[3] - a[3]) / span)
    sound_cpu.append(max(b[4] - a[4], 0) / span)
    busy.append(100 * CORES - (b[5] - a[5]) / span)
relay_seconds = (samples[-1][1] - samples[0][1]) / 100 if len(samples) > 1 else 0
used = [int(x) for x in open(f"{logs}/used.mb").read().split()] if os.path.exists(f"{logs}/used.mb") else []
ms = lambda us: f"{us / 1000:.1f}"


def fanout(label, viewers_log, publisher_log, wanted):
    # viewers.log and, with VIEWER_PROCS, viewers-2.log ... of the other processes.
    stem = viewers_log[: -len(".log")]
    paths = [viewers_log] + sorted(glob.glob(f"{stem}-[0-9]*.log"))
    text = "".join(open(path).read() for path in paths)
    stats = [dict(pair.split("=") for pair in line.split()[1:]) for line in text.splitlines() if line.startswith("VIDEO_STATS")]
    ended = re.findall(r"VIEWERS_ENDED ok=(\d+) failed=(\d+)", text)
    got = [int(s["frames"]) for s in stats]
    before = [int(s["before_the_first_key_frame"]) for s in stats]
    whole = sum(1 for s in stats if int(s["frames"]) + int(s["before_the_first_key_frame"]) >= wanted)
    p50 = [int(s["late_us_p50"]) for s in stats if int(s["frames"])]
    p99 = [int(s["late_us_p99"]) for s in stats if int(s["frames"])]
    mx = [int(s["late_us_max"]) for s in stats if int(s["frames"])]
    published = open(publisher_log).read()
    sent = sum(int(m) for m in re.findall(r"payload_len=(\d+)", published))
    sends = len(re.findall(r"^SENT ", published, re.M))
    # `skipped` of VIDEO_STATS: Objects that came, but not in the order the writer takes them
    # (AnnexBWriter in crates/moqt-client/src/video.rs); lateness is of those it took.
    print("FANOUT %s viewers=%d kbit_per_group=%d viewers_with_all_in_order=%d viewers_ended_ok=%s viewers_failed=%s "
          "taken=%d before_a_key_frame=%d came_out_of_order=%d late_ms_median_of_p50=%s late_ms_worst_p99=%s late_ms_max=%s" % (
        label, viewers, sent * 8 // 1000 // max(sends, 1), whole,
        sum(int(ok) for ok, _ in ended) if ended else "?", sum(int(failed) for _, failed in ended) if ended else "?",
        sum(got), sum(before), sum(int(s["skipped"]) for s in stats),
        ms(statistics.median(p50)) if p50 else "-", ms(max(p99)) if p99 else "-", ms(max(mx)) if mx else "-"))


mean = lambda xs: statistics.mean(xs) if xs else 0
print("RELAY video=%s relay_cpu_mean=%.0f%% relay_cpu_max=%.0f%% relay_cpu_seconds=%.1f relay_rss_max_mb=%.0f "
      "video_viewers_cpu_mean/max=%.0f/%.0f%% sound_viewers_cpu_mean/max=%.0f/%.0f%% machine_busy_mean/max=%.0f/%.0f%%_of_%d00 "
      "machine_used_mb_max=%s samples=%d" % (
    video.split("/")[-1], mean(cpu), max(cpu, default=0), relay_seconds, max(rss, default=0) / 1024,
    mean(video_cpu), max(video_cpu, default=0), mean(sound_cpu), max(sound_cpu, default=0),
    mean(busy), max(busy, default=0), CORES, max(used, default="-"), len(cpu)))
fanout("video", f"{logs}/viewers.log", f"{logs}/publisher.log", frames)
if os.path.exists(f"{logs}/sound-viewers.log"):
    fanout("sound", f"{logs}/sound-viewers.log", f"{logs}/sound-publisher.log", pieces)

relay = open(f"{logs}/relay.log").read()
last = {}
most = {"writing": 0, "waiting_bytes": 0, "waiting_objects": 0, "streams_not_done": 0, "rtt_ms": 0}
for line in relay.splitlines():
    if not line.startswith("STATS "):
        continue
    fields = dict(pair.split("=", 1) for pair in line.split()[1:] if "=" in pair)
    last[fields["session"]] = fields
    for key in most:
        if key in fields:
            most[key] = max(most[key], int(fields[key]))
lost = sum(int(f.get("lost_packets", 0)) for f in last.values())
congestion = sum(int(f.get("congestion_events", 0)) for f in last.values())
# The relay's log is bounded and drops lines when it is written faster than it is read
# (LOG_DROPPED lines=N); STATS lines are among those it drops.
dropped = sum(int(n) for n in re.findall(r"^LOG_DROPPED lines=(\d+)", relay, re.M))
resets = collections.Counter(re.findall(r"^RESET_DATA .*code=(0x[0-9a-f]+)", relay, re.M))
print("STATS_SUMMARY sessions_seen=%d lost_packets_sum=%d congestion_events_sum=%d %s resets=%s "
      "subscriptions_ended_too_far_behind=%d refused=%d handshake_failed=%d log_lines_dropped=%d" % (
    len(last), lost, congestion, " ".join(f"{k}_max={v}" for k, v in most.items()), dict(resets),
    len(re.findall(r"status_code=0x5\b", relay)), relay.count("SESSION_REFUSED"), relay.count("HANDSHAKE_FAILED"), dropped))
