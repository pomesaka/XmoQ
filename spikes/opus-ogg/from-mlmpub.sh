#!/bin/sh
# Writes 5 seconds of an Opus track of mlmpub (moqlivemock) as an Ogg Opus file.
#
#   spikes/opus-ogg/from-mlmpub.sh <out.opus> [objects, 250]        # from the repository root
#
# Needs `mise run interop` once, which builds mlmpub and the client.
set -eu
out=$1
objects=${2:-250}
root=$(cd "$(dirname "$0")/../.." && pwd)
tools="$root/interop/peers/moqlivemock/.tools"
port=${PORT:-48870}
work=$(mktemp -d)
(cd "$root/spikes/opus-ogg" && cargo build --release --quiet)
(cd "$root" && cargo build --quiet -p xmoq-moqt-client)
"$tools/bin/mlmpub" -addr "127.0.0.1:$port" -sideport "$((port + 1000))" -asset "$tools/assets/test10s" \
  -qlog "$work/mlmpub.qlog" > "$work/mlmpub.log" 2>&1 &
publisher=$!
trap 'kill -INT "$publisher" 2>/dev/null || true; rm -rf "$work"' EXIT
fingerprint=
for _ in 1 2 3 4 5 6 7 8 9 10; do
  fingerprint=$(curl -s --max-time 1 "http://127.0.0.1:$((port + 1000))/fingerprint" || true)
  [ -n "$fingerprint" ] && break
  sleep 0.5
done
"$root/target/debug/xmoq-moqt-client" --mode subscribe --objects "$objects" --timeout-secs 60 \
  --url "https://127.0.0.1:$port/moq" --cert-sha256 "$fingerprint" \
  --namespace mlm/msf/clear --track "${TRACK:-audio_monotonic_128kbps_opus}" |
  "$root/spikes/opus-ogg/target/release/opus-ogg" "$out"
