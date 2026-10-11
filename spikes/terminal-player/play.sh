#!/bin/sh
# Plays a broadcast in this terminal: the picture with the Kitty graphics protocol, the sound
# through the machine's output. One command for the broadcast of `mise run demo`:
#
#   spikes/terminal-player/play.sh                 # from the repository root, in Ghostty
#
# after `mise run demo` is up and http://localhost:8080/publish is open in Chromium. Ctrl-C ends
# it. Another broadcast: URL=https://host:port/ CERT_SHA256=<hex> NAMESPACE=a/b play.sh
#
# What does what, all of it an experiment and none of it a choice of a decoder:
#   picture  the own Rust client subscribes to the track "video" and writes Annex B into a FIFO;
#            kitty-frames (spikes/kitty-graphics) decodes it with the openh264 crate and draws it
#   sound    a second session subscribes to the track "audio"; opus-ogg (spikes/opus-ogg) wraps
#            the Opus packets as Ogg, and ffmpeg decodes them and plays them (-f audiotoolbox,
#            which is macOS). SOUND=none for no sound, SOUND=<file.opus> to write the file instead
# Picture and sound are not held to each other's times: each is played as it comes.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
url=${URL:-https://127.0.0.1:4443/}
namespace=${NAMESPACE:-xmoq/demo}
sound=${SOUND:-play}
frames=${FRAMES:-1000000}
if [ -z "${CERT_SHA256:-}" ]; then
  # The certificate `demo/serve.ts` made for its relay.
  CERT_SHA256=$(openssl x509 -in "$root/demo/.cert/cert.pem" -outform der | shasum -a 256 | cut -d' ' -f1)
fi
ffmpeg=$(command -v ffmpeg || echo "$HOME/.local/share/mise/installs/ffmpeg/8.1/bin/ffmpeg")
(cd "$root/spikes/kitty-graphics" && cargo build --release --quiet)
(cd "$root/spikes/opus-ogg" && cargo build --release --quiet)
(cd "$root" && cargo build --quiet -p xmoq-moqt-client)
client="$root/target/debug/xmoq-moqt-client"
work=$(mktemp -d)
mkfifo "$work/video.h264"
trap 'kill "${picture:-}" "${audio:-}" 2>/dev/null || true; rm -rf "$work"' EXIT INT TERM

if [ "$sound" != none ]; then
  (
    "$client" --mode subscribe --objects "$((frames * 2))" --timeout-secs 86400 \
      --url "$url" --cert-sha256 "$CERT_SHA256" --namespace "$namespace" --track audio 2> "$work/audio.log" |
      if [ "$sound" = play ]; then
        "$root/spikes/opus-ogg/target/release/opus-ogg" - 1 2>> "$work/audio.log" |
          "$ffmpeg" -loglevel error -f ogg -i pipe:0 -f audiotoolbox - 2>> "$work/audio.log"
      else
        "$root/spikes/opus-ogg/target/release/opus-ogg" "$sound" 1
      fi
  ) &
  audio=$!
fi
# WHY the one that draws is the one in the background: opening the FIFO waits for the client
# to open it, and a client that is turned down never does. It is stopped then.
"$root/spikes/kitty-graphics/target/release/kitty-frames" --h264 --scale "${SCALE:-2}" \
  --medium "${MEDIUM:-file}" < "$work/video.h264" &
picture=$!
"$client" --mode subscribe --objects "$frames" --timeout-secs 86400 \
  --url "$url" --cert-sha256 "$CERT_SHA256" --namespace "$namespace" --track video \
  --annex-b-out "$work/video.h264" > "$work/video.log" 2>&1 || true
if grep -q VIDEO_WRITTEN "$work/video.log"; then
  wait "$picture" 2>/dev/null || true
  grep VIDEO_WRITTEN "$work/video.log" >&2
else
  echo "no picture: $(tail -1 "$work/video.log")" >&2
fi
wait "${audio:-}" 2>/dev/null || true
