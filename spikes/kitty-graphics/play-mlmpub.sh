#!/bin/sh
# Plays a video track of mlmpub (moqlivemock) in this terminal: the own Rust client subscribes
# and writes Annex B into a FIFO, ffmpeg decodes it to RGB, kitty-frames draws it.
#
#   spikes/kitty-graphics/play-mlmpub.sh [frames] [scale]      # from the repository root
#
# Needs: `mise run interop` once (it builds mlmpub and the client), ffmpeg on the PATH or in
# mise's installs, and a terminal with the Kitty graphics protocol (Ghostty).
set -eu
frames=${1:-250}
scale=${2:-0}
root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/spikes/kitty-graphics"
tools="$root/interop/peers/moqlivemock/.tools"
ffmpeg=$(command -v ffmpeg || echo "$HOME/.local/share/mise/installs/ffmpeg/8.1/bin/ffmpeg")
work=$(mktemp -d)
port=${PORT:-48840}
(cd "$here" && cargo build --release --quiet)
(cd "$root" && cargo build --quiet -p xmoq-moqt-client)
mkfifo "$work/video.h264"
"$tools/bin/mlmpub" -addr "127.0.0.1:$port" -sideport "$((port + 1000))" -asset "$tools/assets/test10s" \
  -qlog "$work/mlmpub.qlog" > "$work/mlmpub.log" 2>&1 &
publisher=$!
trap 'kill -INT "$publisher" 2>/dev/null || true; kill "${client:-}" 2>/dev/null || true; rm -rf "$work"' EXIT
fingerprint=
for _ in 1 2 3 4 5 6 7 8 9 10; do
  fingerprint=$(curl -s --max-time 1 "http://127.0.0.1:$((port + 1000))/fingerprint" || true)
  [ -n "$fingerprint" ] && break
  sleep 0.5
done
"$root/target/debug/xmoq-moqt-client" --mode subscribe --objects "$frames" --timeout-secs 120 \
  --url "https://127.0.0.1:$port/moq" --cert-sha256 "$fingerprint" \
  --namespace mlm/msf/clear --track "${TRACK:-video_400kbps_avc}" \
  --annex-b-out "$work/video.h264" > "$work/client.log" 2>&1 &
client=$!
# Without -probesize and -analyzeduration ffmpeg waits for seconds of input before its first
# frame. NOT -fflags nobuffer: with it 25 of 60 frames of a file did not come out.
"$ffmpeg" -loglevel error -flags low_delay -probesize 32 -analyzeduration 0 \
  -f h264 -i "$work/video.h264" -fps_mode passthrough -f rawvideo -pix_fmt rgb24 pipe:1 |
  "$here/target/release/kitty-frames" --raw 1280x720 --scale "$scale" --medium "${MEDIUM:-direct}"
grep VIDEO_WRITTEN "$work/client.log" >&2 || true
