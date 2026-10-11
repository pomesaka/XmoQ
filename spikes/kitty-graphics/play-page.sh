#!/bin/sh
# Plays the video a page of Chromium encodes and publishes, through the own relay, in this
# terminal: the own Rust client subscribes and writes Annex B into a FIFO, ffmpeg decodes it
# to RGB, kitty-frames draws it.
#
#   spikes/kitty-graphics/play-page.sh [frames] [scale]        # from the repository root
#
# Needs: `mise run interop` once (it builds the relay, the client and the page), ffmpeg on the
# PATH or in mise's installs, and a terminal with the Kitty graphics protocol (Ghostty).
set -eu
frames=${1:-250}
scale=${2:-2}
root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/spikes/kitty-graphics"
ffmpeg=$(command -v ffmpeg || echo "$HOME/.local/share/mise/installs/ffmpeg/8.1/bin/ffmpeg")
work=$(mktemp -d)
(cd "$here" && cargo build --release --quiet)
(cd "$root" && cargo build --quiet -p xmoq-moqt-client -p xmoq-relay)
mkfifo "$work/video.h264"
# WHY the binary and not `mise exec`: the signal at the end has to reach bun itself, which
# stops the page and the relay it started.
bun=$(cd "$root/interop" && mise which bun)
(cd "$root/interop" && exec "$bun" "$here/publish-page.ts" --port "${PORT:-48850}") > "$work/page.log" 2>&1 &
page=$!
trap 'kill -INT "$page" 2>/dev/null || true; kill "${client:-}" 2>/dev/null || true; wait "$page" 2>/dev/null || true; rm -rf "$work"' EXIT
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
"$root/target/debug/xmoq-moqt-client" --mode subscribe --objects "$frames" --timeout-secs 120 \
  --url "$url" --cert-sha256 "$cert" --namespace xmoq/video --track video \
  --annex-b-out "$work/video.h264" > "$work/client.log" 2>&1 &
client=$!
# Without -probesize and -analyzeduration ffmpeg waits for seconds of input before its first
# frame. NOT -fflags nobuffer: with it 25 of 60 frames of a file did not come out.
"$ffmpeg" -loglevel error -flags low_delay -probesize 32 -analyzeduration 0 \
  -f h264 -i "$work/video.h264" -fps_mode passthrough -f rawvideo -pix_fmt rgb24 pipe:1 |
  "$here/target/release/kitty-frames" --raw 320x180 --scale "$scale" --medium "${MEDIUM:-direct}"
grep VIDEO_WRITTEN "$work/client.log" >&2 || true
