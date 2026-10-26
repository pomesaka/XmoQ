#!/bin/sh
# Captures Opus packets a browser encoded, and those of moqlivemock's publisher, as the lines
# of the own Rust client, into spikes/opus-compare/captured/.
#
#   spikes/opus-compare/capture.sh [packets, 9000]           # from the repository root
#
# 9,000 packets of 20 ms are 3 minutes. The browser is the demo's publishing page in headless
# Chromium (a tone, with a beep every 2 seconds), on ports of its own (PORT, 48950, and three
# more). Needs `mise run interop` once (Chromium, mlmpub).
set -eu
packets=${1:-9000}
root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/spikes/opus-compare"
port=${PORT:-48950}
mkdir -p "$here/captured"
(cd "$root" && cargo build --quiet --locked -p xmoq-relay -p xmoq-moqt-client)
client="$root/target/debug/xmoq-moqt-client"
bun=$(cd "$root/interop" && mise which bun)
seconds=$((packets / 50 + 30))
work=$(mktemp -d)
(cd "$root" && exec "$bun" demo/serve.ts --relay-port "$port" --port "$((port + 1))" --rooms-port "$((port + 2))") > "$work/demo.log" 2>&1 &
demo=$!
trap 'kill -INT "${page:-}" "${mlmpub:-}" 2>/dev/null || true; kill -INT "$demo" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$work"' EXIT
for _ in $(seq 1 40); do grep -q "^publish:" "$work/demo.log" && break; sleep 0.5; done
(cd "$root" && exec "$bun" demo/broadcast/scripts/publish-headless.ts "http://localhost:$((port + 1))") > "$work/page.log" 2>&1 &
page=$!
for _ in $(seq 1 40); do grep -q "^PUBLISHING" "$work/page.log" && break; sleep 0.5; done
sha=$(openssl x509 -in "$root/demo/.cert/cert.pem" -outform der | shasum -a 256 | cut -d' ' -f1)
"$client" --mode subscribe --objects "$packets" --timeout-secs "$seconds" --url "https://127.0.0.1:$port/" \
  --cert-sha256 "$sha" --namespace xmoq/demo --track audio > "$here/captured/chromium.packets" 2> "$work/client.log" || true
echo "chromium: $(grep -c '^OBJECT ' "$here/captured/chromium.packets") packets"

tools="$root/interop/peers/moqlivemock/.tools"
"$tools/bin/mlmpub" -addr "127.0.0.1:$((port + 3))" -sideport "$((port + 1003))" -asset "$tools/assets/test10s" \
  -qlog "$work/mlmpub.qlog" > "$work/mlmpub.log" 2>&1 &
mlmpub=$!
fingerprint=
for _ in $(seq 1 20); do
  fingerprint=$(curl -s --max-time 1 "http://127.0.0.1:$((port + 1003))/fingerprint" || true)
  [ -n "$fingerprint" ] && break
  sleep 0.5
done
"$client" --mode subscribe --objects "$packets" --timeout-secs "$seconds" --url "https://127.0.0.1:$((port + 3))/moq" \
  --cert-sha256 "$fingerprint" --namespace mlm/msf/clear --track audio_monotonic_128kbps_opus \
  > "$here/captured/mlmpub.packets" 2>> "$work/client.log" || true
echo "mlmpub: $(grep -c '^OBJECT ' "$here/captured/mlmpub.packets") packets"
