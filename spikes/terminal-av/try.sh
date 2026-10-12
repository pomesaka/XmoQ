#!/bin/sh
# A demo of its own on other ports, its publishing page in headless Chromium, and xmoq-watch
# for some seconds with the picture written to a file nobody looks at and the sound at volume 0.
# Prints AV_PLAYED and what the viewer cost (/usr/bin/time -l).
#
#   spikes/terminal-av/try.sh [seconds, 20] [more flags of xmoq-watch]     # from the repository root
#
# Needs `mise run interop` once (Playwright's Chromium). PORT (48900) and two more are used.
set -eu
seconds=${1:-20}
[ $# -gt 0 ] && shift
root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/spikes/terminal-av"
port=${PORT:-48900}
work=$(mktemp -d)
(cd "$here" && cargo build --release --quiet)
(cd "$root" && cargo build --quiet -p xmoq-relay)
# WHY the binary and not `mise exec`: the signal at the end has to reach bun itself.
bun=$(cd "$root/interop" && mise which bun)
(cd "$root" && exec "$bun" demo/serve.ts --relay-port "$port" --port "$((port + 1))" --rooms-port "$((port + 2))") > "$work/demo.log" 2>&1 &
demo=$!
trap 'kill -INT "${page:-}" 2>/dev/null || true; kill -INT "$demo" 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$work"' EXIT
for _ in $(seq 1 40); do
  grep -q "^publish:" "$work/demo.log" && break
  sleep 0.5
done
grep -q "^publish:" "$work/demo.log" || { cat "$work/demo.log" >&2; exit 1; }
(cd "$root" && exec "$bun" "$here/publish-headless.ts" "http://localhost:$((port + 1))") > "$work/page.log" 2>&1 &
page=$!
for _ in $(seq 1 40); do
  grep -q "^PUBLISHING" "$work/page.log" && break
  sleep 0.5
done
cat "$work/page.log" >&2
sha=$(openssl x509 -in "$root/demo/.cert/cert.pem" -outform der | shasum -a 256 | cut -d' ' -f1)
/usr/bin/time -l "$here/target/release/xmoq-watch" --url "https://127.0.0.1:$port/" --cert-sha256 "$sha" \
  --seconds "$seconds" --volume 0 --medium direct --out "${OUT:-/dev/null}" "$@" 2> "$work/watch.log" || true
grep -E "AV_PLAYED|Error|error|real|maximum resident|peak memory" "$work/watch.log" || cat "$work/watch.log"
