#!/bin/sh
# One run of fanout.sh in a container of its own, removed after it (--rm); after setup.sh.
#   spikes/fanout/linux/run.sh <label> <viewers> <small.h264|large.h264> [SOUND 0|1]
# The logs stay in spikes/fanout/linux/work/logs/<label>/.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
name=${NAME:-xmoq-fanout}
podman=${PODMAN:-podman}
label=$1; viewers=$2; media=$3; sound=${4:-0}
logs="$here/work/logs/$label"
mkdir -p "$logs"
# WHY a memory limit: podman's machine is shared with other containers; this one is stopped
# by the kernel before it takes theirs.
"$podman" run --rm --name "$name-run-$label" --memory 3500m \
  -v "$here:/spike" -v "$name-target:/target:ro" \
  -e LOGS="/spike/work/logs/$label" -e SOUND="$sound" -e MAX_SESSIONS=2100 \
  "localhost/$name-linux" sh /spike/fanout.sh "$viewers" "/spike/work/media/$media" 250
