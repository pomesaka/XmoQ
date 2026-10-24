#!/bin/sh
# Makes what the Linux runs need, on a Mac with podman's machine running (from the root):
#   spikes/fanout/linux/setup.sh
#
# An image ($NAME-linux), two volumes ($NAME-target with the binaries built in it, $NAME-cargo
# for the registry), and in spikes/fanout/linux/work/: the videos (ffmpeg 8.1 with
# VideoToolbox, as on 2026-10-11), a certificate of 13 days, and mlmpub of moqlivemock built
# for linux/arm64. Removed again with:
#   podman image rm localhost/$NAME-linux; podman volume rm $NAME-target $NAME-cargo
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
name=${NAME:-xmoq-fanout}
podman=${PODMAN:-podman}
work="$here/work"
mkdir -p "$work/media" "$work/cert"

"$podman" build -q -t "localhost/$name-linux" -f "$here/Containerfile" "$here"
"$podman" volume exists "$name-target" || "$podman" volume create "$name-target"
"$podman" volume exists "$name-cargo" || "$podman" volume create "$name-cargo"
"$podman" run --rm --name "$name-build" --memory 3g -v "$root:/src:ro" -v "$name-target:/target" \
  -v "$name-cargo:/usr/local/cargo/registry" -e CARGO_TARGET_DIR=/target -w /src "localhost/$name-linux" \
  cargo build --release --locked -j 4 -p xmoq-relay -p xmoq-moqt-client -p xmoq-watch

ff="mise exec ffmpeg@8.1 -- ffmpeg -hide_banner -loglevel error -y"
$ff -f lavfi -i testsrc2=size=320x180:rate=25 -frames:v 100 -c:v h264_videotoolbox -profile:v baseline -b:v 513k -g 25 -bsf:v h264_mp4toannexb -f h264 "$work/media/small.h264"
$ff -f lavfi -i testsrc2=size=1280x720:rate=25 -frames:v 100 -c:v h264_videotoolbox -profile:v baseline -b:v 2500k -g 25 -bsf:v h264_mp4toannexb -f h264 "$work/media/large.h264"
# A track of the shape of the demo's sound: 50 small Objects a second, a Group of 10.
$ff -f lavfi -i testsrc2=size=64x64:rate=50 -frames:v 500 -c:v h264_videotoolbox -profile:v baseline -b:v 67k -g 10 -bsf:v h264_mp4toannexb -f h264 "$work/media/sound-shaped.h264"

openssl ecparam -name prime256v1 -genkey -noout -out "$work/cert/key.pem"
openssl req -x509 -new -key "$work/cert/key.pem" -out "$work/cert/cert.pem" -days 13 -subj /CN=localhost \
  -addext subjectAltName=DNS:localhost,IP:127.0.0.1

# mlmpub, as scripts/ci/interop.sh builds it, for the viewer of watch.sh.
if [ ! -x "$work/mlmpub-linux" ]; then
  source=$(mktemp -d "$work/mlm.XXXXXX")
  git -c advice.detachedHead=false clone --quiet --depth 1 --branch v0.16.1 https://github.com/Eyevinn/moqlivemock "$source"
  [ "$(git -C "$source" rev-parse HEAD)" = 91d903e9933a452f8baaa860f9fcd20cf195e077 ]
  mise exec go@1.26.0 -- env GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go -C "$source" build -o "$work/mlmpub-linux" ./cmd/mlmpub
fi
