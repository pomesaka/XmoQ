#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

# Builds what the interop run needs and runs every pairing (interop/matrix.ts). Arguments are
# passed on to matrix.ts, for example `--only own-rust`.

# The version is part of the path so that a new tag is a new build, here and in the CI cache.
RELAY_TAG="relay@0.15.0"
relay_root="interop/peers/moqtail/.tools"
relay_stamp="${relay_root}/.${RELAY_TAG}"

# WHY a stamp file: `cargo install` from git clones and resolves before it notices that the
# binary is already there. The stamp makes a second run start at once.
if [[ ! -x "${relay_root}/bin/relay" || ! -f "${relay_stamp}" ]]; then
	cargo install --git https://github.com/moqtail/moqtail --tag "${RELAY_TAG}" --locked \
		--root "${relay_root}" relay
	touch "${relay_stamp}"
fi

cargo build --locked --manifest-path interop/peers/moqtail/rust-client/Cargo.toml
cargo build --locked -p xmoq-moqt-client -p xmoq-relay

bun install --frozen-lockfile

# WHY the CLI of the installed package: it downloads the Chromium build that this version of
# playwright-core drives (bun.lock pins the version), and does nothing if it is already there.
# WHY --no-shell: interop/harness.ts launches the full browser (`channel: "chromium"`), not the
# headless shell.
# WHY `bun run` in interop/: it finds the command of the package wherever the package manager put
# it (interop/node_modules or the hoisted node_modules of the root).
playwright_install=(bun run --cwd interop playwright-core install --no-shell)
if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
	# The runner image lacks the shared libraries Chromium needs.
	playwright_install+=(--with-deps)
fi
"${playwright_install[@]}" chromium

bun interop/matrix.ts "$@"
interop_verdict ran
ran "Interop run (every pairing of own and moqtail clients through the moqtail relay and the own relay, and the runs in which a client leaves)"
