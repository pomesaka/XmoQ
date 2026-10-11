#!/bin/sh
# One publisher, the own relay, and N viewers of one video track: what the relay takes of the
# machine, and how late the frames come.
#
#   spikes/fanout/run.sh <viewers> <file.h264> [frames a viewer takes, 250]    # from the root
#
# The publisher is the own Rust client with --video-in: it sends the Groups of the file again
# and again, a Group a second, a frame every 40 ms, each with the wall-clock time as its
# Timestamp. The viewers are sessions of one process (--viewers), which start 10 ms apart.
# Builds with --release. Prints one line, FANOUT ..., and leaves the logs in $LOGS.
set -eu
viewers=$1
video=$2
frames=${3:-250}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo build --release --quiet -p xmoq-moqt-client -p xmoq-relay
port=${PORT:-48860}
logs=${LOGS:-$(mktemp -d)}
mkdir -p "$logs"
cert=interop/.cert/cert.pem
sha=$(openssl x509 -in "$cert" -outform der | shasum -a 256 | cut -d' ' -f1)
common="--url https://127.0.0.1:$port --cert-sha256 $sha --namespace xmoq/fanout --track video"
# The viewers come 10 ms apart, and each takes its frames from then on.
seconds=$((frames / 25 + viewers / 100 + 12))

# MAX_SESSIONS: more than the 256 sessions the relay takes of itself. RELAY_PID_FILE: where
# the PID of the relay is written, for a profiler.
# shellcheck disable=SC2086
target/release/xmoq-relay --port "$port" --cert-file "$cert" --key-file interop/.cert/key.pem \
  ${MAX_SESSIONS:+--max-sessions $MAX_SESSIONS} > "$logs/relay.log" 2>&1 &
relay=$!
echo "$relay" > "$logs/relay.pid"
trap 'kill "${publisher:-}" "${sampler:-}" "$relay" 2>/dev/null || true' EXIT
sleep 1
# shellcheck disable=SC2086
( sleep 2; n=0; while [ "$n" -lt "$seconds" ]; do echo send; n=$((n + 1)); sleep 1; done ) |
  target/release/xmoq-moqt-client $common --mode publish --payload-hex 00 --video-in "$video" \
  --timeout-secs $((seconds + 10)) > "$logs/publisher.log" 2>&1 &
publisher=$!
sleep 3
( while :; do ps -o %cpu=,rss= -p "$relay" >> "$logs/relay.ps" 2>/dev/null || exit 0; sleep 1; done ) &
sampler=$!
# shellcheck disable=SC2086
target/release/xmoq-moqt-client $common --mode subscribe --viewers "$viewers" --objects "$frames" \
  --timeout-secs $((seconds + 5)) > "$logs/viewers.log" 2>&1 || true
kill "$sampler" 2>/dev/null || true
python3 - "$logs" "$viewers" "$video" "$frames" <<'PY'
import re, sys, statistics
logs, viewers, video, frames = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4])
cpu, rss = [], []
for line in open(f"{logs}/relay.ps"):
    c, r = line.split()
    cpu.append(float(c)); rss.append(int(r))
stats = [dict(pair.split("=") for pair in line.split()[1:]) for line in open(f"{logs}/viewers.log") if line.startswith("VIDEO_STATS")]
text = open(f"{logs}/viewers.log").read()
ended = re.search(r"VIEWERS_ENDED ok=(\d+) failed=(\d+)", text)
got = [int(s["frames"]) for s in stats]
before = [int(s["before_the_first_key_frame"]) for s in stats]
whole = sum(1 for s in stats if int(s["frames"]) + int(s["before_the_first_key_frame"]) >= frames)
p50 = [int(s["late_us_p50"]) for s in stats if int(s["frames"])]
p99 = [int(s["late_us_p99"]) for s in stats if int(s["frames"])]
mx = [int(s["late_us_max"]) for s in stats if int(s["frames"])]
ms = lambda us: f"{us / 1000:.1f}"
sent = sum(int(m) for m in re.findall(r"payload_len=(\d+)", open(f"{logs}/publisher.log").read()))
sends = len(re.findall(r"^SENT ", open(f"{logs}/publisher.log").read(), re.M))
print("FANOUT viewers=%d video=%s kbit_s=%d relay_cpu_mean=%.0f%% relay_cpu_max=%.0f%% relay_rss_max_mb=%.0f "
      "viewers_with_every_frame=%d viewers_ended_ok=%s viewers_failed=%s frames=%d waited_for_a_key_frame=%d skipped_after_it=%d "
      "late_ms_median_of_p50=%s late_ms_worst_p99=%s late_ms_max=%s" % (
    viewers, video.split("/")[-1], sent * 8 // 1000 // max(sends, 1),
    statistics.mean(cpu) if cpu else 0, max(cpu, default=0), max(rss, default=0) / 1024,
    whole, ended.group(1) if ended else "?", ended.group(2) if ended else "?",
    sum(got), sum(before), sum(int(s["skipped"]) for s in stats),
    ms(statistics.median(p50)) if p50 else "-", ms(max(p99)) if p99 else "-", ms(max(mx)) if mx else "-"))
PY
