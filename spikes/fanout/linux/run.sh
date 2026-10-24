#!/bin/sh
# One run of fanout.sh in a container of its own, removed after it (--rm); after setup.sh.
#   spikes/fanout/linux/run.sh <label> <viewers> <small.h264|large.h264> [SOUND 0|1]
# PROFILE=1: perf samples the relay (perf.data, perf-self.txt, perf-children.txt).
# BIN=/target/<dir>: other binaries of the volume than /target/release.
# The logs stay in spikes/fanout/linux/work/logs/<label>/.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
name=${NAME:-xmoq-fanout}
podman=${PODMAN:-podman}
label=$1; viewers=$2; media=$3; sound=${4:-0}
logs="$here/work/logs/$label"
mkdir -p "$logs"
# WHY these only with PROFILE=1: podman's seccomp profile and SELinux in its machine (Fedora)
# refuse perf_event_open to a container; with both off for this container, perf samples the
# user space (the machine's perf_event_paranoid is 2). Tried on 2026-10-24.
profile_flags=
if [ "${PROFILE:-0}" = 1 ]; then profile_flags="--security-opt seccomp=unconfined --security-opt label=disable"; fi
# WHY a memory limit: podman's machine is shared with other containers; this one is stopped
# by the kernel before it takes theirs.
# shellcheck disable=SC2086
"$podman" run --rm --name "$name-run-$label" --memory 3500m $profile_flags \
  -v "$here:/spike" -v "$name-target:/target:ro" \
  -e LOGS="/spike/work/logs/$label" -e SOUND="$sound" -e MAX_SESSIONS=2100 \
  -e PROFILE="${PROFILE:-0}" -e BIN="${BIN:-/target/release}" \
  "localhost/$name-linux" sh /spike/fanout.sh "$viewers" "/spike/work/media/$media" 250
