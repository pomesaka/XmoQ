#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

# Builds what the interop run needs and runs every pairing (interop/matrix.ts). Arguments are
# passed on to matrix.ts, for example `--only own-rust`.

# The ports the kernel gives to outgoing sockets. The relays of the run listen on fixed ports,
# which have to lie outside of them (interop/matrix.ts, `base-port`); printed so that a log of
# the run shows it.
if [[ -r /proc/sys/net/ipv4/ip_local_port_range ]]; then
	read -r ephemeral_first ephemeral_last </proc/sys/net/ipv4/ip_local_port_range
	echo "ip_local_port_range: ${ephemeral_first} to ${ephemeral_last}"
fi

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

# moqtail's test client, of the same tag: the publisher of the FETCH pairings, the one here
# that answers a FETCH. A stamp of its own, so that a cache that has the relay alone gets it.
client_stamp="${relay_root}/.client-${RELAY_TAG}"
if [[ ! -x "${relay_root}/bin/client" || ! -f "${client_stamp}" ]]; then
	cargo install --git https://github.com/moqtail/moqtail --tag "${RELAY_TAG}" --locked \
		--root "${relay_root}" client
	touch "${client_stamp}"
fi

# moqlivemock: a publisher that is not ours and has an MSF catalog (interop/catalog.ts).
# WHY a clone and `go build`, where `go install module@tag` would do: its go.mod has a replace
# directive, which `go install` refuses for a module that is not the main one.
# WHY the asset is copied: mlmpub reads the media it serves from a directory, by default one
# relative to its source.
# WHY the commit as well: a tag can be moved, and what is built here runs on this machine.
MOQLIVEMOCK_TAG="v0.16.1"
MOQLIVEMOCK_COMMIT="91d903e9933a452f8baaa860f9fcd20cf195e077"
moqlivemock_root="interop/peers/moqlivemock/.tools"
# WHY a patch: mlmsub sends the CONNECT of WebTransport with the token of a later draft, which
# neither relay here accepts, and has no flag for the other one. The patch is one line and
# changes nothing of MoQ Transport or MSF (interop/peers/moqlivemock/README.md). mlmpub is
# built as it is.
moqlivemock_patch="interop/peers/moqlivemock/mlmsub-legacy-connect-protocol.patch"
# The patch is part of what is built, so a change to it is a new build.
moqlivemock_stamp="${moqlivemock_root}/.${MOQLIVEMOCK_COMMIT}-$(git hash-object "${moqlivemock_patch}")"
if [[ ! -x "${moqlivemock_root}/bin/mlmpub" || ! -f "${moqlivemock_stamp}" ]]; then
	moqlivemock_source="$(mktemp -d)"
	# The clone goes also when a step below fails.
	trap 'rm -rf "${moqlivemock_source}"' EXIT
	git -c advice.detachedHead=false clone --quiet --depth 1 --branch "${MOQLIVEMOCK_TAG}" \
		https://github.com/Eyevinn/moqlivemock "${moqlivemock_source}"
	moqlivemock_cloned="$(git -C "${moqlivemock_source}" rev-parse HEAD)"
	if [[ "${moqlivemock_cloned}" != "${MOQLIVEMOCK_COMMIT}" ]]; then
		fail "the tag ${MOQLIVEMOCK_TAG} of moqlivemock is ${moqlivemock_cloned}, not ${MOQLIVEMOCK_COMMIT}: it was moved, or this script is out of date"
	fi
	rm -rf "${moqlivemock_root}"
	mkdir -p "${moqlivemock_root}/bin" "${moqlivemock_root}/assets"
	(cd "${moqlivemock_source}" && go build -o "${REPO_ROOT}/${moqlivemock_root}/bin/" ./cmd/mlmpub)
	# Fails when the patch no longer fits the source.
	git -C "${moqlivemock_source}" apply "${REPO_ROOT}/${moqlivemock_patch}"
	(cd "${moqlivemock_source}" && go build -o "${REPO_ROOT}/${moqlivemock_root}/bin/" ./cmd/mlmsub)
	cp -R "${moqlivemock_source}/assets/test10s" "${moqlivemock_root}/assets/test10s"
	rm -rf "${moqlivemock_source}"
	trap - EXIT
	touch "${moqlivemock_stamp}"
fi

cargo build --locked --manifest-path interop/peers/moqtail/rust-client/Cargo.toml
need_alsa_headers
cargo build --locked -p xmoq-moqt-client -p xmoq-relay -p xmoq-quiet-peer -p xmoq-chat
# WHY --release for this one: interop/harness.ts, at `TERMINAL_VIEWER_BINARY`.
cargo build --locked --release -p xmoq-watch

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
