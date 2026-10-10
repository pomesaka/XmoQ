#!/usr/bin/env bash
# Sends one object through the relay and checks that the receiver got the same bytes.
#
#   scripts/transfer.sh <direction> <label> [version]
#
#   direction  b2r  browser publishes, Rust subscribes
#              r2b  Rust publishes, browser subscribes
#              b2b  browser publishes, browser subscribes
#   label      names the broadcast and the log files under logs/
#   version    moq-lite-NN or moq-transport-NN; omit for each library's default offer
#
# Needs a running relay (scripts/relay.sh), a built rust-client, `bun install` in web/,
# and CHROMIUM_PATH pointing at a Chromium or Chrome binary.
set -euo pipefail

DIRECTION="$1"
LABEL="$2"
VERSION="${3:-}"

cd "$(dirname "$0")/.."
mkdir -p logs

RELAY_URL="http://127.0.0.1:4443/spike"
RUST="rust-client/target/debug/xmoq-spike-moq-dev"
# Random per run, so a match cannot come from an earlier run's cached object.
PAYLOAD="xmoq-${LABEL}-${RANDOM}${RANDOM}"

RUST_VERSION=()
WEB_VERSION=()
if [ -n "${VERSION}" ]; then
	RUST_VERSION=(--connect-version "${VERSION}")
	# The browser pins by WebTransport subprotocol, which is the ALPN of the version.
	case "${VERSION}" in
		moq-lite-0[12]) ALPN="moql" ;;
		moq-lite-*) ALPN="${VERSION}" ;;
		moq-transport-14) ALPN="moq-00" ;;
		moq-transport-*) ALPN="moqt-${VERSION#moq-transport-}" ;;
		*) echo "unknown version: ${VERSION}" >&2; exit 1 ;;
	esac
	WEB_VERSION=(--protocols "${ALPN}")
fi

rust() {
	# WHY the WebSocket fallback is off: the spike must prove the QUIC path.
	"${RUST}" --connect "${RELAY_URL}" --connect-websocket-enabled=false \
		--broadcast "${LABEL}" --payload "${PAYLOAD}" \
		${RUST_VERSION[@]+"${RUST_VERSION[@]}"} "$@"
}

web() {
	bun run ./web/run.ts --relay "${RELAY_URL}" --broadcast "${LABEL}" --payload "${PAYLOAD}" \
		${WEB_VERSION[@]+"${WEB_VERSION[@]}"} "$@"
}

PUB_LOG="logs/${LABEL}-publisher.log"
SUB_LOG="logs/${LABEL}-subscriber.log"

# LIVE_INTERVAL_MS (optional): the browser publisher rewrites the payload as a new group
# every N ms. Unset, the object is written once before the subscriber connects (late join).
WEB_LIVE=()
if [ -n "${LIVE_INTERVAL_MS:-}" ]; then
	WEB_LIVE=(--interval "${LIVE_INTERVAL_MS}")
fi

case "${DIRECTION}" in
	b2r | b2b) web --role publish --hold 20 ${WEB_LIVE[@]+"${WEB_LIVE[@]}"} >"${PUB_LOG}" 2>&1 & ;;
	r2b) rust --wait 20 publish >"${PUB_LOG}" 2>&1 & ;;
	*) echo "unknown direction: ${DIRECTION}" >&2; exit 1 ;;
esac
PUB_PID=$!
# PUB_PID is the subshell running the function; the publisher itself is its child.
trap 'pkill -P "${PUB_PID}" 2>/dev/null; kill "${PUB_PID}" 2>/dev/null || true' EXIT

# Give the publisher time to connect and announce. The subscribers also wait for the announcement.
sleep 5

STATUS=0
case "${DIRECTION}" in
	b2r) rust --wait 12 subscribe >"${SUB_LOG}" 2>&1 || STATUS=$? ;;
	r2b | b2b) web --role subscribe --timeout 12 >"${SUB_LOG}" 2>&1 || STATUS=$? ;;
esac

echo "== ${DIRECTION} ${LABEL} version=${VERSION:-default} payload=${PAYLOAD} exit=${STATUS}"
echo "-- publisher (${PUB_LOG})"
perl -pe 's/\e\[[0-9;]*m//g' "${PUB_LOG}"
echo "-- subscriber (${SUB_LOG})"
perl -pe 's/\e\[[0-9;]*m//g' "${SUB_LOG}"
exit "${STATUS}"
