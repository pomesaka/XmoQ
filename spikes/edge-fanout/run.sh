#!/bin/sh
# One publisher at an origin relay, two edge relays below it (--parent), and N viewers of one
# video track at each edge; or, with EDGES=0, the 2N viewers at the origin itself, to compare.
# What reaches the origin, how many copies of each Group it sends, what each relay takes of the
# machine, and how late the frames come. As spikes/fanout/run.sh, which it follows.
#
#   spikes/edge-fanout/run.sh <viewers at each edge> <file.h264> [frames a viewer takes, 250]
#
# Run from the root. PORT (34500) is the origin's; the edges take the two after it. LOGS is
# where the logs go. Builds with --release. Prints one line, EDGE_FANOUT ...
set -eu
viewers=$1
video=$2
frames=${3:-250}
edges=${EDGES:-2}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo build --release --quiet -p xmoq-moqt-client -p xmoq-relay
port=${PORT:-34500}
logs=${LOGS:-$(mktemp -d)}
mkdir -p "$logs"
cert=interop/.cert/cert.pem
key=interop/.cert/key.pem
sha=$(openssl x509 -in "$cert" -outform der | shasum -a 256 | cut -d' ' -f1)
track="--cert-sha256 $sha --namespace xmoq/fanout --track video"
seconds=$((frames / 25 + viewers / 100 + 12))
pids=""
trap 'kill $pids 2>/dev/null || true' EXIT

# More sessions than the 256 a relay takes of itself. A GOAWAY Timeout of 0, as in the interop
# run: a relay told to stop sends GOAWAY and refuses connections for that long, still holding
# its port, and the next run's relay could not bind it.
relay() { # <port> <log> [flags]
  p=$1; l=$2; shift 2
  target/release/xmoq-relay --port "$p" --cert-file "$cert" --key-file "$key" \
    --max-sessions 4096 --goaway-timeout-ms 0 "$@" > "$logs/$l.log" 2>&1 &
  pids="$pids $!"
  echo "$!" > "$logs/$l.pid"
}
relay "$port" origin
n=1
while [ "$n" -le "$edges" ]; do
  relay $((port + n)) "edge$n" --parent "https://127.0.0.1:$port" --parent-cert-sha256 "$sha"
  n=$((n + 1))
done
sleep 1
# shellcheck disable=SC2086
( sleep 2; s=0; while [ "$s" -lt "$seconds" ]; do echo send; s=$((s + 1)); sleep 1; done ) |
  target/release/xmoq-moqt-client --url "https://127.0.0.1:$port" $track --mode publish \
  --payload-hex 00 --video-in "$video" --timeout-secs $((seconds + 10)) > "$logs/publisher.log" 2>&1 &
pids="$pids $!"
sleep 3
for l in origin $(n=1; while [ "$n" -le "$edges" ]; do echo "edge$n"; n=$((n + 1)); done); do
  r=$(cat "$logs/$l.pid")
  ( while :; do ps -o %cpu=,rss= -p "$r" >> "$logs/$l.ps" 2>/dev/null || exit 0; sleep 1; done ) &
  pids="$pids $!"
done
# The viewers: N at each edge, or 2N at the origin with EDGES=0.
watchers=""
if [ "$edges" -eq 0 ]; then
  # shellcheck disable=SC2086
  target/release/xmoq-moqt-client --url "https://127.0.0.1:$port" $track --mode subscribe \
    --viewers $((viewers * 2)) --objects "$frames" --timeout-secs $((seconds + 5)) \
    > "$logs/viewers0.log" 2>&1 &
  watchers="$!"
else
  n=1
  while [ "$n" -le "$edges" ]; do
    # shellcheck disable=SC2086
    target/release/xmoq-moqt-client --url "https://127.0.0.1:$((port + n))" $track \
      --mode subscribe --viewers "$viewers" --objects "$frames" --timeout-secs $((seconds + 5)) \
      > "$logs/viewers$n.log" 2>&1 &
    watchers="$watchers $!"
    n=$((n + 1))
  done
fi
for w in $watchers; do wait "$w" || true; done
python3 - "$logs" "$viewers" "$video" "$frames" "$edges" <<'PY'
import glob, re, sys, statistics
logs, viewers, video, frames, edges = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4]), int(sys.argv[5])
def ps(name):
    cpu, rss = [], []
    for line in open(f"{logs}/{name}.ps"):
        c, r = line.split(); cpu.append(float(c)); rss.append(int(r))
    return (statistics.mean(cpu) if cpu else 0, max(cpu, default=0), max(rss, default=0) / 1024)
stats, ok, failed = [], 0, 0
for path in sorted(glob.glob(f"{logs}/viewers*.log")):
    text = open(path).read()
    stats += [dict(p.split("=") for p in l.split()[1:]) for l in text.splitlines() if l.startswith("VIDEO_STATS")]
    m = re.search(r"VIEWERS_ENDED ok=(\d+) failed=(\d+)", text)
    if m: ok += int(m.group(1)); failed += int(m.group(2))
whole = sum(1 for s in stats if int(s["frames"]) + int(s["before_the_first_key_frame"]) >= frames)
p50 = [int(s["late_us_p50"]) for s in stats if int(s["frames"])]
p99 = [int(s["late_us_p99"]) for s in stats if int(s["frames"])]
mx = [int(s["late_us_max"]) for s in stats if int(s["frames"])]
ms = lambda us: f"{us / 1000:.1f}"
def groups(name):
    # Subgroup streams the relay opened, by Group: the copies of each Group it sends.
    text = open(f"{logs}/{name}.log").read()
    by = {}
    for g in re.findall(r"^OPEN_DATA session=\d+ out=\d+ track_alias=\d+ group=(\d+)", text, re.M):
        by[g] = by.get(g, 0) + 1
    counts = sorted(by.values())
    return (statistics.median(counts) if counts else 0, len(re.findall(r"^PARENT_CONNECTED", text, re.M)),
            len(re.findall(r"^SESSION_OPENED", text, re.M)))
o = ps("origin"); og = groups("origin")
line = ("EDGE_FANOUT edges=%d viewers_at_each=%d viewers=%d frames=%d origin_sessions=%d origin_copies_a_group=%s "
        "origin_cpu_mean=%.0f%% origin_cpu_max=%.0f%% origin_rss_max_mb=%.0f" % (
    edges, viewers, len(stats), frames, og[2], og[0], *o))
for n in range(1, edges + 1):
    e = ps(f"edge{n}"); eg = groups(f"edge{n}")
    line += (" edge%d_upstream_sessions=%d edge%d_copies_a_group=%s edge%d_cpu_mean=%.0f%% edge%d_cpu_max=%.0f%% edge%d_rss_max_mb=%.0f"
             % (n, eg[1], n, eg[0], n, e[0], n, e[1], n, e[2]))
line += (" viewers_with_every_frame=%d viewers_ended_ok=%d viewers_failed=%d late_ms_median_of_p50=%s late_ms_worst_p99=%s late_ms_max=%s" % (
    whole, ok, failed, ms(statistics.median(p50)) if p50 else "-", ms(max(p99)) if p99 else "-", ms(max(mx)) if mx else "-"))
print(line)
PY
