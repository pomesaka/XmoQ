"""Where an Object's time goes between its Timestamp and the relay's hub, for the two tracks
of a run with `PUBLISHER_FLAGS='--log-written on'` and `RELAY_FLAGS='--log-arrivals on'`:

- opened: from the Timestamp to when its stream was open (the publisher waited for the
  relay to allow one more stream, or for the stream of its Group, opened earlier),
- written: from the Timestamp to when the transport took its bytes,
- arrived: from the Timestamp to when the relay's hub took it (one clock: one machine).

  python3 arrivals.py <logs>
"""
import os, re, sys

logs = sys.argv[1]


def written(path):
    out = {}
    if not os.path.exists(path):
        return out
    for line in open(path):
        if line.startswith("WRITTEN "):
            d = dict(p.split("=") for p in line.split()[1:] if "=" in p)
            out[(int(d["group"]), int(d["object"]))] = (int(d["timestamp_us"]), int(d["opened_us"]), int(d["written_us"]))
            if "rtt_ms" in d:
                second = int(d["timestamp_us"]) // 1_000_000
                paths.setdefault(path, {}).setdefault(second, []).append((int(d["rtt_ms"]), int(d["cwnd"])))
    return out


# For each publisher, second by second: its connection's round trip and congestion window.
paths = {}


tracks = {"video": written(f"{logs}/publisher.err"), "sound": written(f"{logs}/sound-publisher.err")}
# The relay's session of each publisher: the one whose streams are of Subgroup Zero is the video's.
streams, arrived = {}, {}
for line in open(f"{logs}/relay.err", errors="replace"):
    # Lines of other writers to stderr may be cut into these; such a line is left out.
    d = dict(p.split("=", 1) for p in line.split()[1:] if "=" in p)
    if line.startswith("ARRIVED_STREAM ") and {"session", "stream", "subgroup", "group"} <= d.keys():
        track = "video" if d["subgroup"] == "Zero" else "sound"
        streams[(d["session"], d["stream"])] = (track, int(d["group"]))
    elif line.startswith("ARRIVED ") and {"session", "stream", "object", "at_us"} <= d.keys() and d["at_us"].isdigit():
        key = streams.get((d["session"], d["stream"]))
        if key:
            arrived[(key[0], key[1], int(d["object"]))] = int(d["at_us"])


def q(xs, p):
    xs = sorted(xs)
    return xs[int(p * (len(xs) - 1))] if xs else float("nan")


for track, objects in tracks.items():
    if not objects:
        continue
    opened = [(o - t) / 1000 for (t, o, _) in objects.values()]
    wrote = [(w - t) / 1000 for (t, _, w) in objects.values()]
    came = [(arrived[(track, g, o)] - t) / 1000 for (g, o), (t, _, _) in objects.items() if (track, g, o) in arrived]
    print(f"ARRIVALS {track} objects={len(objects)} at_the_relay={len(came)}")
    for name, xs in (("opened", opened), ("written", wrote), ("arrived", came)):
        print(f"  {name:8s} ms after the Timestamp: p50={q(xs, .5):.1f} p99={q(xs, .99):.1f} max={max(xs, default=float('nan')):.1f}")
    # Second by second, how late the Objects were at the relay (the run's course).
    by_second = {}
    for (g, o), (t, _, _) in objects.items():
        if (track, g, o) in arrived:
            by_second.setdefault(t // 1_000_000, []).append((arrived[(track, g, o)] - t) / 1000)
    first = min(by_second, default=0)
    print("  arrived, max ms a second: " + " ".join(f"{s - first}:{max(v):.0f}" for s, v in sorted(by_second.items())))
    path = paths.get(f"{logs}/publisher.err" if track == "video" else f"{logs}/sound-publisher.err", {})
    if path:
        print("  the publisher's connection, max rtt ms / min cwnd a second: "
              + " ".join(f"{s - first}:{max(r for r, _ in v)}/{min(c for _, c in v)}" for s, v in sorted(path.items())))
