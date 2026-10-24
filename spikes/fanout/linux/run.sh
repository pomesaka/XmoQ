#!/bin/sh
# One run of fanout.sh in a container of its own, removed after it (--rm); after setup.sh.
#   spikes/fanout/linux/run.sh <label> <viewers> <small.h264|large.h264> [SOUND 0|1]
# PROFILE=1: perf samples the relay (perf.data, perf-groups.txt and perf's own reports).
# BIN=/target/<dir>: other binaries of the volume than /target/release.
# RELAY_FLAGS: more flags of the relay ("--writers session"), split at blanks there.
# SPLIT=1: the relay on cores of its own (RELAY_CPUS, 0-1), the publishers and the viewers
# on the others (CLIENTS_CPUS, 2-4), by taskset.
# The logs stay in spikes/fanout/linux/work/logs/<label>/.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
name=${NAME:-xmoq-fanout}
podman=${PODMAN:-podman}
label=$1; viewers=$2; media=$3; sound=${4:-0}
mkdir -p "$here/work/logs/$label"
# WHY these only with PROFILE=1: podman's seccomp profile and SELinux in its machine (Fedora)
# refuse perf_event_open to a container; with both off for this container, perf samples the
# user space (the machine's perf_event_paranoid is 2). Tried on 2026-10-24.
profile_flags=
if [ "${PROFILE:-0}" = 1 ]; then profile_flags="--security-opt seccomp=unconfined --security-opt label=disable"; fi
# WHY taskset and not --cpuset-cpus: rootless podman has no cpuset controller in that
# machine (crun: "controller `cpuset` is not available", 2026-10-24).
cpus=
if [ "${SPLIT:-0}" = 1 ]; then cpus="-e RELAY_CPUS=${RELAY_CPUS:-0-1} -e CLIENTS_CPUS=${CLIENTS_CPUS:-2-4}"; fi
relay_flags=
if [ -n "${RELAY_FLAGS:-}" ]; then relay_flags="-e RELAY_FLAGS"; fi
# WHY a memory limit: podman's machine is shared with other containers; this one is stopped
# by the kernel before it takes theirs.
# shellcheck disable=SC2086
"$podman" run --rm --name "$name-run-$label" --memory 3500m $profile_flags $cpus $relay_flags \
  -v "$here:/spike" -v "$name-target:/target:ro" \
  -e LOGS="/spike/work/logs/$label" -e SOUND="$sound" -e MAX_SESSIONS=2100 \
  -e PROFILE="${PROFILE:-0}" -e BIN="${BIN:-/target/release}" \
  "localhost/$name-linux" sh /spike/fanout.sh "$viewers" "/spike/work/media/$media" 250
