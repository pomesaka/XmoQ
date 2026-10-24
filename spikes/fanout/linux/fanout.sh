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
bin=${BIN:-/target/release}
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

# RELAY_CPUS and CLIENTS_CPUS (run.sh SPLIT=1): the cores the relay, and the publishers and
# viewers, run on. taskset executes the program, so $! is still its PID.
on_relay=; on_clients=
if [ -n "${RELAY_CPUS:-}" ]; then on_relay="taskset -c $RELAY_CPUS"; fi
if [ -n "${CLIENTS_CPUS:-}" ]; then on_clients="taskset -c $CLIENTS_CPUS"; fi
# shellcheck disable=SC2086
$on_relay "$bin/xmoq-relay" --port "$port" --cert-file "$cert" --key-file /spike/work/cert/key.pem \
  --max-sessions "${MAX_SESSIONS:-256}" --stats-every-ms 1000 ${RELAY_FLAGS:-} > "$logs/relay.log" 2> "$logs/relay.err" &
relay=$!
trap 'for p in ${publisher:-} ${sound:-} ${sampler:-} ${sound_viewers:-} ${video_viewers:-} ${profiler:-} $relay; do case "$p" in ""|0|*[!0-9]*) ;; *) kill "$p" 2>/dev/null || true;; esac; done' EXIT
sleep 1
# shellcheck disable=SC2086
( sleep 2; n=0; while [ "$n" -lt "$seconds" ]; do echo send; n=$((n + 1)); sleep 1; done ) |
  $on_clients "$bin/xmoq-moqt-client" $common --track video --mode publish --payload-hex 00 --video-in "$video" \
  --timeout-secs $((seconds + 10)) ${PUBLISHER_FLAGS:-} > "$logs/publisher.log" 2> "$logs/publisher.err" &
publisher=$!
if [ "${SOUND:-0}" = 1 ]; then
  # A Group of 10 pieces every 200 ms: one line, one Group.
  # shellcheck disable=SC2086
  ( sleep 2; n=0; while [ "$n" -lt $((seconds * 5)) ]; do echo send; n=$((n + 1)); sleep 0.2; done ) |
    $on_clients "$bin/xmoq-moqt-client" $common --track audio --mode publish --payload-hex 00 \
    --video-in /spike/work/media/sound-shaped.h264 --video-frame-ms 20 --video-streams object \
    --timeout-secs $((seconds + 10)) ${PUBLISHER_FLAGS:-} > "$logs/sound-publisher.log" 2> "$logs/sound-publisher.err" &
  sound=$!
fi
sleep 3
# VIEWER_PROCS=k: the viewers of each track in k processes of viewers/k sessions each (the
# logs of the others are viewers-2.log ...). VIEWER_FLAGS: more flags of the viewers.
procs=${VIEWER_PROCS:-1}
each=$((viewers / procs))
sound_viewers=; video_viewers=
k=1
while [ "$k" -le "$procs" ]; do
  suffix=; [ "$k" -gt 1 ] && suffix="-$k"
  if [ "${SOUND:-0}" = 1 ]; then
    # shellcheck disable=SC2086
    $on_clients "$bin/xmoq-moqt-client" $common --track audio --mode subscribe --viewers "$each" --objects "$pieces" \
      --timeout-secs $((seconds + 5)) ${VIEWER_FLAGS:-} > "$logs/sound-viewers$suffix.log" 2>&1 &
    sound_viewers="$sound_viewers $!"
  fi
  # shellcheck disable=SC2086
  $on_clients "$bin/xmoq-moqt-client" $common --track video --mode subscribe --viewers "$each" --objects "$frames" \
    --timeout-secs $((seconds + 5)) ${VIEWER_FLAGS:-} > "$logs/viewers$suffix.log" 2>&1 &
  video_viewers="$video_viewers $!"
  k=$((k + 1))
done
# WHY /proc and not ps: the %cpu of Linux's ps is over the whole life of the process, where
# macOS's is of the last moments. Every second: the time, utime+stime in ticks (100 a second)
# of the relay, the viewers of the video and those of the sound, the relay's RSS in KiB, and
# the ticks the machine's cores were idle (/proc/stat), which tell whether the 5 were all busy.
# The ticks of the processes named, summed; 0 once the first of them is gone.
ticks() {
  [ -e "/proc/${1%% *}/stat" ] || { echo 0; return; }
  sum=0
  for p in $1; do sum=$((sum + $(awk '{print $14 + $15}' "/proc/$p/stat" 2>/dev/null || echo 0))); done
  echo "$sum"
}
( while :; do
    [ -e "/proc/$relay/stat" ] || exit 0
    rss=$(awk '/^VmRSS/ {print $2}' "/proc/$relay/status")
    idle=$(awk '/^cpu / {print $5}' /proc/stat)
    echo "$(date +%s.%N) $(ticks "$relay") $rss $(ticks "${video_viewers# }") $(ticks "${sound_viewers# }") $idle" >> "$logs/relay.ticks"
    free -m | awk '/^Mem/ {print $3}' >> "$logs/used.mb"
    sleep 1
  done ) &
sampler=$!
# The UDP counters of the container's network (RcvbufErrors: datagrams the kernel dropped
# for a full receive buffer), before the viewers and after.
grep '^Udp:' /proc/net/snmp > "$logs/udp-before.txt"
# How long the machine itself stops (stalls.py), while the viewers run.
python3 /spike/stalls.py "$seconds" > "$logs/stalls.txt" 2>&1 &
# The threads of the relay and of the viewers' processes, five times a second apart, once
# the viewers have all begun: whether one of them is busy all the time.
( sleep $((viewers / 100 + 2))
  top -H -b -d 1 -n 5 -w 120 -p "$(echo "$relay $video_viewers $sound_viewers" | tr -s ' ' ',' | sed 's/,$//')" > "$logs/threads.txt" 2>&1 ) &
if [ "${PROFILE:-0}" = 1 ]; then
  # PROFILE=1: perf samples the relay for 10 s once the viewers have all begun (they begin
  # 10 ms apart), the user space alone (perf_event_paranoid 2 of podman's machine).
  ( sleep $((viewers / 100 + 1))
    perf record -q -e cpu-clock:u -F 499 --call-graph dwarf,16384 -p "$relay" -o "$logs/perf.data" -- sleep 10 \
      > "$logs/perf-record.log" 2>&1 ) &
  profiler=$!
fi
for p in $video_viewers; do wait "$p" || true; done
grep '^Udp:' /proc/net/snmp > "$logs/udp-after.txt"
if [ -n "${profiler:-}" ]; then
  wait "$profiler" || true
  # What each function takes itself, and with what it calls (the hub's own work is under
  # Hub::run and the like).
  perf report -i "$logs/perf.data" --no-children --sort symbol --stdio --percent-limit 0.3 -g none > "$logs/perf-self.txt" 2>/dev/null || true
  perf report -i "$logs/perf.data" --children --sort symbol --stdio --percent-limit 1 -g none > "$logs/perf-children.txt" 2>/dev/null || true
  # The stacks, demangled (setup.sh installs rustfilt), grouped by perf-groups.py.
  perf script -i "$logs/perf.data" -F ip,sym --no-inline 2>/dev/null | /target/tools/bin/rustfilt > "$logs/perf.demangled"
  python3 /spike/perf-groups.py < "$logs/perf.demangled" > "$logs/perf-groups.txt"
fi
for p in $sound_viewers; do wait "$p" || true; done
kill "$sampler" 2>/dev/null || true
python3 /spike/summary.py "$logs" "$viewers" "$video" "$frames" "$pieces"
