#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

if [[ ! -f Cargo.toml ]]; then
	if compgen -G "crates/*/Cargo.toml" >/dev/null; then
		fail "crates/ contains crates but there is no root Cargo.toml, so nothing would check them"
	fi
	skip "Rust checks" "no root Cargo.toml yet"
	exit 0
fi

# WHY here and not in the workflow: `xmoq-watch` plays sound through cpal, whose ALSA backend
# (`alsa-sys`) asks pkg-config for the headers of ALSA when it is built for Linux. Whoever
# runs this check needs them, in CI or not. macOS needs nothing more.
if [[ "$(uname -s)" == Linux ]] && ! pkg-config --exists alsa 2>/dev/null; then
	if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
		sudo apt-get update
		sudo apt-get install --yes --no-install-recommends libasound2-dev pkg-config
	else
		fail "the headers of ALSA are missing (Debian and Ubuntu: libasound2-dev and pkg-config), which xmoq-watch needs to build"
	fi
fi

# WHY --workspace everywhere: spikes/ is excluded from the workspace, so these commands never
# reach it. Spike crates pin old library versions to reproduce one experiment and are not
# maintained to the lint level of the main crates.
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
# WHY once more without features: `xmoq-moqt-io` has to build and pass with the in-memory
# connection alone, and nothing else in the workspace builds it that way.
cargo test -p xmoq-moqt-io --no-default-features --locked
ran "Rust checks (fmt, clippy, test)"
