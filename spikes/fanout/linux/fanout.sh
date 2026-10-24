#!/bin/sh
# spikes/fanout/run.sh, in a Linux container (run.sh starts it): one publisher, the own relay,
# N viewers of one video track; with SOUND=1 also a publisher of a track of the shape of the
# demo's sound (an Object every 20 ms, each on its own stream) and N more viewers of it, in a
# process and a session of their own: one viewer is two sessions.
#
#   fanout.sh <viewers> <file.h264> [frames a viewer takes, 250]
#
# Binaries in /target/release (setup.sh), this directory at /spike. The relay writes a STATS
# line for every session every second. Prints RELAY, FANOUT and STATS_SUMMARY lines.
set -eu
viewers=$1
video=$2
frames=${3:-250}
bin=/target/release
port=48860
logs=${LOGS:-$(mktemp -d)}
mkdir -p "$logs"
# A label run again begins with empty files.
: > "$logs/relay.ticks"
: > "$logs/used.mb"
rm -f "$logs/sound-viewers.log" "$logs/sound-publisher.log"
cert=/spike/work/cert/cert.pem
sha=$(openssl x509 -in "$cert" -outform der | sha256sum | cut -d' ' -f1)
common="--url https://127.0.0.1:$port --cert-sha256 $sha --namespace xmoq/fanout"
seconds=$((frames / 25 + viewers / 100 + 12))
pieces=$((frames * 2))

"$bin/xmoq-relay" --port "$port" --cert-file "$cert" --key-file /spike/work/cert/key.pem \
  --max-sessions "${MAX_SESSIONS:-256}" --stats-every-ms 1000 > "$logs/relay.log" 2>&1 &
relay=$!
trap 'for p in ${publisher:-} ${sound:-} ${sampler:-} ${sound_viewers:-} $relay; do case "$p" in ""|0|*[!0-9]*) ;; *) kill "$p" 2>/dev/null || true;; esac; done' EXIT
sleep 1
# shellcheck disable=SC2086
( sleep 2; n=0; while [ "$n" -lt "$seconds" ]; do echo send; n=$((n + 1)); sleep 1; done ) |
  "$bin/xmoq-moqt-client" $common --track video --mode publish --payload-hex 00 --video-in "$video" \
  --timeout-secs $((seconds + 10)) > "$logs/publisher.log" 2>&1 &
publisher=$!
if [ "${SOUND:-0}" = 1 ]; then
  # A Group of 10 pieces every 200 ms: one line, one Group.
  # shellcheck disable=SC2086
  ( sleep 2; n=0; while [ "$n" -lt $((seconds * 5)) ]; do echo send; n=$((n + 1)); sleep 0.2; done ) |
    "$bin/xmoq-moqt-client" $common --track audio --mode publish --payload-hex 00 \
    --video-in /spike/work/media/sound-shaped.h264 --video-frame-ms 20 --video-streams object \
    --timeout-secs $((seconds + 10)) > "$logs/sound-publisher.log" 2>&1 &
  sound=$!
fi
sleep 3
# WHY /proc and not ps: the %cpu of Linux's ps is over the whole life of the process, where
# macOS's is of the last moments. utime+stime in ticks (100 a second), and the RSS in KiB.
( while :; do
    stat=$(cat "/proc/$relay/stat" 2>/dev/null) || exit 0
    rss=$(awk '/^VmRSS/ {print $2}' "/proc/$relay/status")
    echo "$(date +%s.%N) $(echo "$stat" | awk '{print $14 + $15}') $rss" >> "$logs/relay.ticks"
    free -m | awk '/^Mem/ {print $3}' >> "$logs/used.mb"
    sleep 1
  done ) &
sampler=$!
if [ "${SOUND:-0}" = 1 ]; then
  # shellcheck disable=SC2086
  "$bin/xmoq-moqt-client" $common --track audio --mode subscribe --viewers "$viewers" --objects "$pieces" \
    --timeout-secs $((seconds + 5)) > "$logs/sound-viewers.log" 2>&1 &
  sound_viewers=$!
fi
# shellcheck disable=SC2086
"$bin/xmoq-moqt-client" $common --track video --mode subscribe --viewers "$viewers" --objects "$frames" \
  --timeout-secs $((seconds + 5)) > "$logs/viewers.log" 2>&1 || true
if [ -n "${sound_viewers:-}" ]; then wait "$sound_viewers" || true; fi
kill "$sampler" 2>/dev/null || true
python3 /spike/summary.py "$logs" "$viewers" "$video" "$frames" "$pieces"
