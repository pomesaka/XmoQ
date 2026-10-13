#!/bin/sh
# One publisher, the own relay, a link that delays and loses between the relay and the viewers,
# and how late the frames of a video come to them.
#
#   spikes/latency-loss/run.sh <delay ms, one way> <loss, relay to viewer, 0..1> <file.h264> \
#       <group|object> [frames a viewer takes, 500] [viewers, 4]              # from the root
#
# The link is spikes/lossy-link/proxy.mjs, between the relay and the viewers alone: the
# publisher is connected to the relay itself. Prints one line, LATENCY ..., made by analyse.py
# from what the viewers say of every Object that came (VIDEO_ARRIVALS). Builds with --release.
set -eu
delay=$1
loss=$2
video=$3
streams=$4
frames=${5:-500}
viewers=${6:-4}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo build --release --quiet -p xmoq-moqt-client -p xmoq-relay
port=${PORT:-48710}
link=$((port + 1))
control=$((port + 2))
logs=${LOGS:-$(mktemp -d)}
mkdir -p "$logs"
cert=interop/.cert/cert.pem
sha=$(openssl x509 -in "$cert" -outform der | shasum -a 256 | cut -d' ' -f1)
seconds=$((frames / 25 + 14))
node=$(command -v node || (cd interop && mise which bun))

# PATIENCE_MS: the relay gives up a Group a viewer does not have that long after its last Object
# (--delivery-patience-ms). Without it no Group is given up.
patience=${PATIENCE_MS:+--delivery-patience-ms $PATIENCE_MS}
# CONGESTION=cubic|newreno|bbr: the congestion control of the relay (cubic without it).
# --slow-open-ms 0: a line for every stream the relay opens, with its congestion window.
target/release/xmoq-relay --port "$port" --cert-file "$cert" --key-file interop/.cert/key.pem \
  --congestion "${CONGESTION:-cubic}" --slow-open-ms 0 $patience > "$logs/relay.log" 2>&1 &
relay=$!
"$node" spikes/lossy-link/proxy.mjs --listen "$link" --to "$port" --control "$control" > "$logs/link.log" 2>&1 &
proxy=$!
trap 'kill "${publisher:-}" "$proxy" "$relay" 2>/dev/null || true' EXIT
sleep 1
# RATE (kbit/s, 0 for no limit) and QUEUE (ms of datagrams that may wait): a link that has no
# more room than that, each way, shared by the viewers. What does not fit in the queue is dropped.
curl -s "http://127.0.0.1:$control/set?loss=0&up=0&down=$loss&delay=$delay&jitter=0&rate=${RATE:-0}&queue=${QUEUE:-100}" > /dev/null
# FRAME_MS and GROUP_SECONDS: for a file that is no video of 25 frames a second with a Group a
# second (20 and 0.2 for the shape of audio: 50 Objects a second, 10 to a Group).
frame_ms=${FRAME_MS:-40}
group_seconds=${GROUP_SECONDS:-1}
sends=$(python3 -c "print(int($seconds / $group_seconds))")
( sleep 2; n=0; while [ "$n" -lt "$sends" ]; do echo send; n=$((n + 1)); sleep "$group_seconds"; done ) |
  target/release/xmoq-moqt-client --url "https://127.0.0.1:$port" --cert-sha256 "$sha" \
  --namespace xmoq/latency --track video --mode publish --payload-hex 00 --video-in "$video" \
  --video-streams "$streams" --video-frame-ms "$frame_ms" --timeout-secs $((seconds + 10)) > "$logs/publisher.log" 2>&1 &
publisher=$!
sleep 3
target/release/xmoq-moqt-client --url "https://127.0.0.1:$link" --cert-sha256 "$sha" \
  --namespace xmoq/latency --track video --mode subscribe --viewers "$viewers" --objects "$frames" \
  --timeout-secs $((seconds + 5)) > "$logs/viewers.log" 2>&1 || true
curl -s "http://127.0.0.1:$control/stats" > "$logs/link-stats.json" || true
FRAME_MS=$frame_ms python3 spikes/latency-loss/analyse.py "$logs/viewers.log" "$delay" "$loss" "$(basename "$video")" "$streams" \
  "${CONGESTION:-cubic}" "$logs/relay.log" "$logs/publisher.log"
