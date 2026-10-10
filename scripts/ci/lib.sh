# shellcheck shell=bash
# Sourced by the check-*.sh scripts.

# shellcheck disable=SC2034 # read by the scripts that source this file
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

# WHY a notice and a summary line: a skipped check exits 0 like a passed one. Without a visible
# marker, a green job could not be told apart from a job that checked nothing.
skip() {
	local what="$1" why="$2"
	echo "SKIP: ${what}: ${why}"
	if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
		echo "::notice title=${what} skipped::${why}"
		echo "- **${what}: skipped.** ${why}" >>"${GITHUB_STEP_SUMMARY}"
	fi
}

ran() {
	local what="$1"
	echo "OK: ${what}"
	if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
		echo "- ${what}: ran" >>"${GITHUB_STEP_SUMMARY}"
	fi
}
