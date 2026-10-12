#!/bin/sh
# Plays the broadcast of `mise run demo` in this terminal, without a room: the relay is the
# demo's, and the hash of its certificate is read from the file the demo made.
#
#   crates/watch/scripts/watch-demo.sh [flags of xmoq-watch]       # from the repository root
#
# after `mise run demo` is up and http://localhost:8080/publish is open in Chromium. With the
# demo's rooms there is no need for this: `mise run watch -- --room http://127.0.0.1:8787/rooms/demo`.
# Another relay: URL=https://host:port/ CERT_SHA256=<hex> NAMESPACE=a/b watch-demo.sh
set -eu
root=$(cd "$(dirname "$0")/../../.." && pwd)
if [ -z "${CERT_SHA256:-}" ]; then
  CERT_SHA256=$(openssl x509 -in "$root/demo/.cert/cert.pem" -outform der | shasum -a 256 | cut -d' ' -f1)
fi
cd "$root"
exec cargo run --release --quiet --locked -p xmoq-watch -- --url "${URL:-https://127.0.0.1:4443/}" \
  --cert-sha256 "$CERT_SHA256" --namespace "${NAMESPACE:-xmoq/demo}" "$@"
