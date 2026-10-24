#!/bin/sh
# The relay and the publishers in a container of podman's machine, the viewers outside it, on
# this Mac (the Rust client of target/release, built for macOS), through the UDP port podman
# forwards; after setup.sh and `cargo build --release -p xmoq-moqt-client`.
#
#   spikes/fanout/linux/outside.sh <label> <rooms> <viewers a room> [frames, 250]
#
# Each room has a process of <viewers> sessions for the video and one for the sound. Prints the
# lines of summary.py, without the machine's cores (the viewers are not in the VM), and the
# memory of this Mac before and after (vm_stat).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
name=${NAME:-xmoq-fanout}
podman=${PODMAN:-podman}
label=$1; rooms=$2; viewers=$3; frames=${4:-250}
logs="$here/work/logs/$label"
mkdir -p "$logs"
rm -f "$logs"/viewers*.log "$logs"/sound-viewers*.log "$logs/ready"
seconds=$((frames / 25 + viewers / 100 + 30))
pieces=$((frames * 2))
client="$root/target/release/xmoq-moqt-client"
sha=$(openssl x509 -in "$here/work/cert/cert.pem" -outform der | shasum -a 256 | cut -d' ' -f1)
# WHY 127.0.0.1 and this port: rootless podman forwards a published port of the Mac's loopback
# to the container, through its gvproxy.
"$podman" run -d --rm --name "$name-outside-$label" --memory 3500m -p 127.0.0.1:48860:48860/udp \
  -v "$here:/spike" -v "$name-target:/target:ro" -e LOGS="/spike/work/logs/$label" -e ROOMS="$rooms" \
  "localhost/$name-linux" sh /spike/relay-and-publishers.sh /spike/work/media/small.h264 "$seconds" > /dev/null
trap '"$podman" stop -t 5 "$name-outside-$label" > /dev/null 2>&1 || true' EXIT
n=0
while [ ! -e "$logs/ready" ] && [ "$n" -lt 30 ]; do sleep 1; n=$((n + 1)); done
sleep 3
vm_stat > "$logs/vm_stat-before.txt"
pids=
r=1
while [ "$r" -le "$rooms" ]; do
  ns=xmoq/fanout; suffix=
  if [ "$rooms" -gt 1 ]; then ns="xmoq/fanout/$r"; [ "$r" -gt 1 ] && suffix="-$r-1"; fi
  common="--url https://127.0.0.1:48860 --cert-sha256 $sha --namespace $ns"
  # shellcheck disable=SC2086
  "$client" $common --track audio --mode subscribe --viewers "$viewers" --objects "$pieces" \
    --timeout-secs $((seconds - 5)) > "$logs/sound-viewers$suffix.log" 2>&1 &
  pids="$pids $!"
  # shellcheck disable=SC2086
  "$client" $common --track video --mode subscribe --viewers "$viewers" --objects "$frames" \
    --timeout-secs $((seconds - 5)) > "$logs/viewers$suffix.log" 2>&1 &
  pids="$pids $!"
  r=$((r + 1))
done
# The memory of this Mac while they run: once a second, the free and compressed pages.
( while :; do vm_stat | awk -v t="$(date +%s)" '/Pages free/ {f=$3} /occupied by compressor/ {c=$5} /Swapouts/ {s=$2} END {gsub("\\.","",f); gsub("\\.","",c); gsub("\\.","",s); print t, f, c, s}' >> "$logs/mac-memory.txt"; sleep 1; done ) &
sampler=$!
for p in $pids; do wait "$p" || true; done
case "$sampler" in ""|0|*[!0-9]*) ;; *) kill "$sampler" 2>/dev/null || true;; esac
touch "$logs/viewers.done"
sleep 2
vm_stat > "$logs/vm_stat-after.txt"
python3 "$here/summary.py" "$logs" "$((viewers * rooms))" small.h264 "$frames" "$pieces" | sed 's/ machine_busy[^ ]*//'
# Pages of 16 KiB: the least free, the most compressed, and the swapouts during the run.
awk 'NR == 1 {s0 = $4} {if (min == "" || $2 < min) min = $2; if ($3 > max) max = $3; s = $4} END {printf "MAC_MEMORY free_mb_least=%d compressed_mb_most=%d swapouts=%d\n", min * 16 / 1024, max * 16 / 1024, s - s0}' "$logs/mac-memory.txt"
