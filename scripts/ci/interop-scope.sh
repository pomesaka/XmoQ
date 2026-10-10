#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

# Decides whether a pull request can change the outcome of the interop run, and writes
# `run=true` or `run=false` to the step outputs.
#
# WHY a step instead of `paths:` on the workflow: a job that a path filter leaves out reports
# nothing, and a required check that reports nothing looks like one that is still pending. This
# way the job always ends, and says in its summary whether it ran the pairings or skipped them.

# Everything the run builds or reads. A change anywhere else cannot alter its result.
paths=(
	crates packages interop fixtures
	.github scripts/ci mise.toml rust-toolchain.toml
	Cargo.toml Cargo.lock package.json bun.lock
)

decide() {
	echo "run=$1" >>"${GITHUB_OUTPUT}"
}

# BASE_SHA is set for pull requests only. A push to main always runs.
if [[ -z "${BASE_SHA:-}" ]]; then
	decide true
	exit 0
fi

changed="$(git diff --name-only "${BASE_SHA}...HEAD" -- "${paths[@]}")"
if [[ -n "${changed}" ]]; then
	echo "Changed files that the interop run depends on:"
	echo "${changed}"
	decide true
else
	skip "Interop run" "this pull request changes nothing under: ${paths[*]}"
	interop_verdict skipped
	decide false
fi
