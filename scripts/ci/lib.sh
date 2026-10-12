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

# Records how the interop job ended, for scripts/ci/interop-verdict.sh. Outside GitHub Actions
# there is no job to account for, and nothing is written.
interop_verdict() {
	if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
		echo "$1" >"${RUNNER_TEMP}/interop-verdict"
	fi
}

ran() {
	local what="$1"
	echo "OK: ${what}"
	if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
		echo "- ${what}: ran" >>"${GITHUB_STEP_SUMMARY}"
	fi
}

# WHY here and not in the workflow: `xmoq-watch` plays sound through cpal, whose ALSA backend
# (`alsa-sys`) asks pkg-config for the headers of ALSA when it is built for Linux. Whoever
# builds it needs them, in CI or not: the Rust checks and the interop runs both do. macOS needs
# nothing more.
need_alsa_headers() {
	if [[ "$(uname -s)" != Linux ]] || pkg-config --exists alsa 2>/dev/null; then
		return
	fi
	if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
		sudo apt-get update
		sudo apt-get install --yes --no-install-recommends libasound2-dev pkg-config
	else
		fail "the headers of ALSA are missing (Debian and Ubuntu: libasound2-dev and pkg-config), which xmoq-watch needs to build"
	fi
}
