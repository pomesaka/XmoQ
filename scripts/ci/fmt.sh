#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

if [[ -f Cargo.toml ]]; then
	cargo fmt --all
fi
if compgen -G "packages/*/package.json" >/dev/null; then
	biome check --write
fi
