#!/bin/sh
# The relay and the publishers of fanout.sh alone, in the container, for viewers outside it
# (outside.sh starts both): ROOMS rooms, each a namespace with a publisher of <file.h264> and
# one of the sound-shaped track. It runs until $LOGS/viewers.done appears, or <seconds> pass.
#
#   relay-and-publishers.sh <file.h264> <seconds>
set -eu
video=$1
seconds=$2
bin=${BIN:-/target/release}
port=48860
logs=$LOGS
mkdir -p "$logs"
: > "$logs/relay.ticks"
rm -f "$logs/viewers.done"
cert=/spike/work/cert/cert.pem
sha=$(openssl x509 -in "$cert" -outform der | sha256sum | cut -d' ' -f1)
"$bin/xmoq-relay" --port "$port" --cert-file "$cert" --key-file /spike/work/cert/key.pem \
  --max-sessions 2100 --stats-every-ms 1000 > "$logs/relay.log" 2> "$logs/relay.err" &
relay=$!
pids=
trap 'for p in $pids $relay; do case "$p" in ""|0|*[!0-9]*) ;; *) kill "$p" 2>/dev/null || true;; esac; done' EXIT
sleep 1
rooms=${ROOMS:-1}
r=1
while [ "$r" -le "$rooms" ]; do
  room=; ns=xmoq/fanout
  if [ "$rooms" -gt 1 ]; then ns="xmoq/fanout/$r"; [ "$r" -gt 1 ] && room="-$r"; fi
  common="--url https://127.0.0.1:$port --cert-sha256 $sha --namespace $ns"
  # shellcheck disable=SC2086
  ( n=0; while [ "$n" -lt "$seconds" ]; do echo send; n=$((n + 1)); sleep 1; done ) |
    "$bin/xmoq-moqt-client" $common --track video --mode publish --payload-hex 00 --video-in "$video" \
    --timeout-secs $((seconds + 10)) > "$logs/publisher$room.log" 2>&1 &
  pids="$pids $!"
  # shellcheck disable=SC2086
  ( n=0; while [ "$n" -lt $((seconds * 5)) ]; do echo send; n=$((n + 1)); sleep 0.2; done ) |
    "$bin/xmoq-moqt-client" $common --track audio --mode publish --payload-hex 00 \
    --video-in /spike/work/media/sound-shaped.h264 --video-frame-ms 20 --video-streams object \
    --timeout-secs $((seconds + 10)) > "$logs/sound-publisher$room.log" 2>&1 &
  pids="$pids $!"
  r=$((r + 1))
done
echo READY > "$logs/ready"
# As fanout.sh's sampler; the columns of the viewers are 1 and 0 (they are outside), so that
# summary.py takes every second.
n=0
while [ ! -e "$logs/viewers.done" ] && [ "$n" -lt "$seconds" ]; do
  rss=$(awk '/^VmRSS/ {print $2}' "/proc/$relay/status")
  idle=$(awk '/^cpu / {print $5}' /proc/stat)
  echo "$(date +%s.%N) $(awk '{print $14 + $15}' "/proc/$relay/stat") $rss 1 0 $idle" >> "$logs/relay.ticks"
  grep '^Udp:' /proc/net/snmp | tail -1 > "$logs/udp-now.txt"
  sleep 1
  n=$((n + 1))
done
