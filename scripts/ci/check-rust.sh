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

# WHY --workspace everywhere: spikes/ is excluded from the workspace, so these commands never
# reach it. Spike crates pin old library versions to reproduce one experiment and are not
# maintained to the lint level of the main crates.
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
ran "Rust checks (fmt, clippy, test)"
