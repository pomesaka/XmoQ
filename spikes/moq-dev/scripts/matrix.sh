#!/usr/bin/env bash
# Runs transfer.sh repeatedly against a relay pinned to one version, for several versions.
#
#   scripts/matrix.sh <repetitions> <version>...
#
# For each version it starts its own relay, runs b2r, r2b and b2b <repetitions> times
# each, prints one line per run, and stops the relay. Full logs land in logs/.
#
# TAG (optional) is added to the labels and log names, so a second pass does not
# overwrite the first. LIVE_INTERVAL_MS is passed through to transfer.sh.
set -uo pipefail

REPS="$1"
shift

cd "$(dirname "$0")/.."
mkdir -p logs

for VERSION in "$@"; do
	bash scripts/relay.sh "${VERSION}" >"logs/relay-${VERSION}${TAG:+-${TAG}}.log" 2>&1 &
	RELAY_PID=$!

	READY=0
	for _ in $(seq 1 50); do
		if curl -s -m 1 -o /dev/null http://127.0.0.1:4443/certificate.sha256; then
			READY=1
			break
		fi
		sleep 0.2
	done
	if [ "${READY}" -ne 1 ]; then
		echo "== relay did not start for ${VERSION}; see logs/relay-${VERSION}${TAG:+-${TAG}}.log"
		kill "${RELAY_PID}" 2>/dev/null
		continue
	fi

	for DIRECTION in b2r r2b b2b; do
		for REP in $(seq 1 "${REPS}"); do
			bash scripts/transfer.sh "${DIRECTION}" "${VERSION}${TAG:+-${TAG}}-${DIRECTION}-${REP}" "${VERSION}" 2>&1 | grep '^== '
		done
	done

	kill "${RELAY_PID}" 2>/dev/null
	wait "${RELAY_PID}" 2>/dev/null
	# The UDP port is not always free the instant the process exits.
	sleep 2
done
