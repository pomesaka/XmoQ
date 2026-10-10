#!/usr/bin/env bash
# Runs moq-relay in the foreground on 127.0.0.1:4443 (QUIC on UDP, HTTP on TCP).
#
#   scripts/relay.sh                      # every version the relay supports
#   scripts/relay.sh moq-transport-22     # accept only this version
#
# Provisional, for local validation only: anonymous access to everything and a
# generated self-signed certificate. Neither is a decision about XmoQ's auth or TLS.
set -euo pipefail

cd "$(dirname "$0")/.."
RELAY="$(find bin -type f -name moq-relay | head -n 1)"
if [ -z "${RELAY}" ]; then
	echo "moq-relay not found; run scripts/fetch-relay.sh first" >&2
	exit 1
fi

VERSION_ARGS=()
if [ $# -ge 1 ]; then
	VERSION_ARGS=(--listen-version "$1")
fi

exec "${RELAY}" \
	--listen 127.0.0.1:4443 \
	--listen-tls-generate localhost,127.0.0.1 \
	--web-http-listen 127.0.0.1:4443 \
	--auth-public "**" \
	--log-level debug \
	${VERSION_ARGS[@]+"${VERSION_ARGS[@]}"}
