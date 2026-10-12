#!/bin/sh
# Plays the tone a page of Chromium encodes as Opus and publishes, through the own relay, on
# the machine's output: the own Rust client subscribes, opus-play decodes and plays.
#
#   spikes/rust-audio/play-page.sh [objects, 500] [volume, 0]        # from the repository root
#
# The volume is 0 unless given: the run makes no sound and says what the output was handed
# (AUDIO_PLAYED), and what the player cost (/usr/bin/time -l). FEATURES=libopus builds the
# player with libopus instead of the decoder in Rust (needs cmake).
# Needs `mise run interop` once (it builds the relay, the client and the page).
set -eu
objects=${1:-500}
volume=${2:-0}
root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/spikes/rust-audio"
work=$(mktemp -d)
(cd "$here" && cargo build --release --quiet ${FEATURES:+--features "$FEATURES"})
(cd "$root" && cargo build --quiet -p xmoq-moqt-client -p xmoq-relay)
# WHY the binary and not `mise exec`: the signal at the end has to reach bun itself, which
# stops the page and the relay it started.
bun=$(cd "$root/interop" && mise which bun)
(cd "$root/interop" && exec "$bun" "$root/spikes/kitty-graphics/publish-page.ts" --port "${PORT:-48860}") > "$work/page.log" 2>&1 &
page=$!
trap 'kill -INT "$page" 2>/dev/null || true; wait "$page" 2>/dev/null || true; rm -rf "$work"' EXIT
ready=
for _ in $(seq 1 60); do
  ready=$(grep -o "READY .*" "$work/page.log" || true)
  [ -n "$ready" ] && break
  if grep -q "FAILED" "$work/page.log"; then cat "$work/page.log" >&2; exit 1; fi
  sleep 0.5
done
[ -n "$ready" ] || { cat "$work/page.log" >&2; exit 1; }
url=$(echo "$ready" | sed -E 's/.*url=([^ ]+).*/\1/')
cert=$(echo "$ready" | sed -E 's/.*cert=([^ ]+).*/\1/')
"$root/target/debug/xmoq-moqt-client" --mode subscribe --objects "$objects" --timeout-secs 120 \
  --url "$url" --cert-sha256 "$cert" --namespace xmoq/video --track audio 2> "$work/client.log" |
  /usr/bin/time -l "$here/target/release/opus-play" --channels "${CHANNELS:-1}" --volume "$volume" 2> "$work/time.log"
grep -E "real|maximum resident|peak memory" "$work/time.log" || cat "$work/time.log"
