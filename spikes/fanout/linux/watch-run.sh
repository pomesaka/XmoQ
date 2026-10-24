#!/bin/sh
# watch.sh in a container of its own, removed after it (--rm); after setup.sh and once
# `mise run interop` (for moqlivemock's media).
#   spikes/fanout/linux/watch-run.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
name=${NAME:-xmoq-fanout}
podman=${PODMAN:-podman}
"$podman" run --rm --name "$name-watch" --memory 1g \
  -v "$here:/spike" -v "$name-target:/target:ro" \
  -v "$root/interop/peers/moqlivemock/.tools/assets:/assets:ro" \
  "localhost/$name-linux" sh /spike/watch.sh
