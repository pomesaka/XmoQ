#!/bin/sh
# The viewer behind a link with less room than the video: the demo's publishing page in
# headless Chromium sends a picture of 2.5 Mbit/s, and between the relay and the viewer is
# spikes/lossy-link/proxy.mjs with a rate, a queue and a delay. With --video-timeout-ms the
# viewer asks the relay to give up Groups of the video that are late.
#
#   crates/watch/scripts/try-narrow-link.sh [seconds, 25] [flags of xmoq-watch]   # from the root
#
# PUBLISH_QUERY is the publishing page's (bitrate=2500000&picture=busy); with &ladder=1 it has
# a second, small version of the video, which --video-version chooses.
# RATE (kbit/s, 2000), QUEUE (ms, 100) and DELAY (ms each way, 50) are the link's. The picture
# goes to /dev/null and the sound is not opened. Prints AV_PLAYED and what the link dropped.
# Needs `mise run interop` once (Playwright's Chromium). PORT (48960) and four more are used.
set -eu
seconds=${1:-25}
[ $# -gt 0 ] && shift
root=$(cd "$(dirname "$0")/../../.." && pwd)
here="$root/crates/watch/scripts"
port=${PORT:-48960}
link=$((port + 3))
control=$((port + 4))
work=$(mktemp -d)
(cd "$root" && cargo build --release --quiet --locked -p xmoq-watch && cargo build --quiet --locked -p xmoq-relay)
bun=$(cd "$root/interop" && mise which bun)
node=$(command -v node || echo "$bun")
(cd "$root" && exec "$bun" demo/serve.ts --no-auth --relay-port "$port" --port "$((port + 1))" --rooms-port "$((port + 2))") > "$work/demo.log" 2>&1 &
demo=$!
trap 'kill -INT "${page:-}" 2>/dev/null || true; kill "${proxy:-}" 2>/dev/null || true; kill -INT "$demo" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$work"' EXIT
for _ in $(seq 1 40); do grep -q "^publish:" "$work/demo.log" && break; sleep 0.5; done
grep -q "^publish:" "$work/demo.log" || { cat "$work/demo.log" >&2; exit 1; }
"$node" "$root/spikes/lossy-link/proxy.mjs" --listen "$link" --to "$port" --control "$control" > "$work/link.log" 2>&1 &
proxy=$!
sleep 1
curl -s "http://127.0.0.1:$control/set?loss=0&delay=${DELAY:-50}&rate=${RATE:-2000}&queue=${QUEUE:-100}" > /dev/null
# A picture that fills the bitrate it asks for, at more than the link has room for.
(cd "$root" && PUBLISH_QUERY="${PUBLISH_QUERY:-bitrate=2500000&picture=busy}" exec "$bun" "$here/publish-headless.ts" "http://localhost:$((port + 1))") > "$work/page.log" 2>&1 &
page=$!
for _ in $(seq 1 40); do grep -q "^PUBLISHING" "$work/page.log" && break; sleep 0.5; done
sha=$(openssl x509 -in "$root/demo/.cert/cert.pem" -outform der | shasum -a 256 | cut -d' ' -f1)
"$root/target/release/xmoq-watch" --url "https://127.0.0.1:$link/" --cert-sha256 "$sha" --namespace xmoq/demo \
  --seconds "$seconds" --sound off --medium direct --fit none --out /dev/null "$@" 2> "$work/watch.log" || true
grep -E "AV_PLAYED|Error|SESSION_ENDED" "$work/watch.log" || cat "$work/watch.log"
echo "LINK $(curl -s "http://127.0.0.1:$control/stats")"
