#!/bin/sh
# Plays the broadcast of `mise run demo` in this terminal, picture and sound by one clock:
#
#   spikes/terminal-av/watch.sh                 # from the repository root, in Ghostty
#
# after `mise run demo` is up and http://localhost:8080/publish is open in Chromium. Ctrl-C ends
# it and prints what was played (AV_PLAYED). More flags of xmoq-watch go after the name:
#   spikes/terminal-av/watch.sh --volume 0.3 --scale 2 --buffer-ms 150
# Another broadcast: URL=https://host:port/ CERT_SHA256=<hex> NAMESPACE=a/b watch.sh
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
if [ -z "${CERT_SHA256:-}" ]; then
  # The certificate `demo/serve.ts` made for its relay.
  CERT_SHA256=$(openssl x509 -in "$root/demo/.cert/cert.pem" -outform der | shasum -a 256 | cut -d' ' -f1)
fi
(cd "$root/spikes/terminal-av" && cargo build --release --quiet)
exec "$root/spikes/terminal-av/target/release/xmoq-watch" --url "${URL:-https://127.0.0.1:4443/}" \
  --cert-sha256 "$CERT_SHA256" --namespace "${NAMESPACE:-xmoq/demo}" "$@"
