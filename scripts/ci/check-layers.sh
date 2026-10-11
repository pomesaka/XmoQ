#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

# Crates that must stay free of I/O, as "package name:directory".
SANS_IO_CRATES=("xmoq-moqt:crates/moqt" "xmoq-relay-core:crates/relay-core")

# WHY the socket-layer crates and not only the transports: banning web-transport-quinn and quinn by name
# would miss the next transport crate. mio, socket2, polling, async-io and quinn-udp sit under
# the async runtimes, so a dependency that does network I/O through tokio, smol or async-std
# brings one of them in. tokio itself is judged the same way: with tokio 1.49.0 the "net"
# feature pulls in mio and socket2 and "sync" alone pulls in neither (checked with cargo tree).
# This cannot see a crate that calls std::net or libc directly.
# WHY NOT an allowlist of permitted dependencies: it would have to be edited for every harmless
# crate (bytes, thiserror), and the review of that edit is the same judgement this list encodes.
IO_CRATES=(mio socket2 polling async-io quinn-udp web-transport-quinn quinn)

errors=0
report() {
	echo "FAIL: $*" >&2
	errors=$((errors + 1))
}

check_rust() {
	local metadata members manifest
	metadata="$(cargo metadata --format-version 1 --no-deps --locked)"
	members="$(jq -r '.packages[].manifest_path' <<<"${metadata}")"

	while IFS= read -r manifest; do
		if [[ "${manifest}" == "${REPO_ROOT}/spikes/"* || "${manifest}" == "${REPO_ROOT}/interop/"* ]]; then
			report "${manifest} is a workspace member; spikes/ and interop/ must stay outside the workspace"
		fi
		# WHY: [workspace.lints] only applies to crates that opt in. A crate without this
		# line compiles with no project lints and clippy still reports success.
		if ! awk '
			/^\[/ { section = $0 }
			section == "[lints]" && /^workspace[ \t]*=[ \t]*true/ { found = 1 }
			/^lints\.workspace[ \t]*=[ \t]*true/ { found = 1 }
			END { exit !found }
		' "${manifest}"; then
			report "${manifest} does not inherit the workspace lints; add [lints] workspace = true"
		fi
	done <<<"${members}"

	shopt -s nullglob
	local crate_manifests=(crates/*/Cargo.toml)
	shopt -u nullglob
	for manifest in "${crate_manifests[@]}"; do
		if ! grep -Fxq "${REPO_ROOT}/${manifest}" <<<"${members}"; then
			report "${manifest} is not a workspace member, so fmt, clippy and test skip it"
		fi
	done

	# WHY: a crate under the workspace root that is neither a member nor excluded cannot be
	# built at all ("current package believes it's in a workspace when it's not"), which would
	# break the spikes that the experiment records tell readers to run, and the moqtail peers
	# under interop/ that the own components are checked against.
	local outside_root outside_dirs=(spikes)
	if [[ -d interop ]]; then
		outside_dirs+=(interop)
	fi
	while IFS= read -r manifest; do
		if ! outside_root="$(cargo metadata --format-version 1 --no-deps --manifest-path "${manifest}" 2>/dev/null | jq -r '.workspace_root')"; then
			report "${manifest} no longer resolves; the root Cargo.toml needs exclude = [\"spikes\", \"interop\"]"
		elif [[ "${outside_root}" == "${REPO_ROOT}" ]]; then
			report "${manifest} resolves into the root workspace; spikes/ and interop/ must stay outside it"
		fi
	done < <(find "${outside_dirs[@]}" -name Cargo.toml -not -path '*/target/*' -not -path '*/.tools/*' -not -path '*/node_modules/*')

	local spike_deps
	spike_deps="$(jq -r --arg spikes "${REPO_ROOT}/spikes/" \
		'.packages[] | .name as $pkg | .dependencies[] | select(.path != null and (.path | startswith($spikes))) | "\($pkg) -> \(.path)"' \
		<<<"${metadata}")"
	if [[ -n "${spike_deps}" ]]; then
		report "workspace crates depend on spikes/: ${spike_deps}"
	fi

	local entry name dir tree banned before
	for entry in "${SANS_IO_CRATES[@]}"; do
		name="${entry%%:*}"
		dir="${entry#*:}"
		if ! jq -e --arg name "${name}" '.packages[] | select(.name == $name)' <<<"${metadata}" >/dev/null; then
			if [[ -e "${dir}" ]]; then
				report "${dir} exists but the workspace has no package named ${name}; update SANS_IO_CRATES in this script"
			else
				skip "I/O-free check for ${name}" "${dir} does not exist yet"
			fi
			continue
		fi
		# WHY normal and build edges only: tests may drive the crate with real sockets or a
		# runtime; the rule is about what users of the crate are forced to link.
		# WHY --all-features: an optional I/O dependency is still an I/O dependency.
		tree="$(cargo tree --package "${name}" --edges normal,build --all-features --prefix none --format '{p}' --locked | awk '{print $1}' | sort -u)"
		before=${errors}
		for banned in "${IO_CRATES[@]}"; do
			if grep -Fxq "${banned}" <<<"${tree}"; then
				report "${name} must not perform I/O but depends on ${banned}; see the path with 'cargo tree --edges normal,build --all-features --invert ${banned}'"
			fi
		done
		if [[ ${errors} -eq ${before} ]]; then
			ran "I/O-free check for ${name}"
		fi
	done
}

# Dependencies of a package.json as "name: version" lines.
package_deps() {
	jq -r \
		'[.dependencies, .devDependencies, .peerDependencies, .optionalDependencies] | map(select(. != null) | to_entries[]) | .[] | "\(.key): \(.value)"' \
		"$1"
}

check_spike_deps() {
	local manifest="$1" spike_deps
	spike_deps="$(package_deps "${manifest}" | grep -F 'spikes/' || true)"
	if [[ -n "${spike_deps}" ]]; then
		report "${manifest} depends on spikes/: ${spike_deps}"
	fi
}

check_packages() {
	local manifest interop_deps interop_name
	interop_name=""
	if [[ -f interop/package.json ]]; then
		interop_name="$(jq -r '.name' interop/package.json)"
	fi
	for manifest in "$@"; do
		check_spike_deps "${manifest}"
		# WHY by name as well as by path: in the Bun workspace a package would reach interop/
		# as "<name>: workspace:*", with no path in it.
		interop_deps="$(package_deps "${manifest}" | grep -E "interop/|^${interop_name:-<none>}: " || true)"
		if [[ -n "${interop_deps}" ]]; then
			report "${manifest} depends on interop/, which checks packages and is not part of them: ${interop_deps}"
		fi
	done
	# WHY: MoQ Transport is the lower layer. @xmoq/msf may use @xmoq/moqt; a transport that
	# knew the streaming format on top of it could no longer carry another one.
	if [[ -f packages/moqt/package.json ]]; then
		local format_deps
		format_deps="$(package_deps packages/moqt/package.json | grep -E '^@xmoq/msf: |packages/msf' || true)"
		if [[ -n "${format_deps}" ]]; then
			report "packages/moqt/package.json depends on @xmoq/msf, which is the layer above it: ${format_deps}"
		fi
	fi
}

checked=0
if [[ -f Cargo.toml ]]; then
	check_rust
	checked=1
elif compgen -G "crates/*/Cargo.toml" >/dev/null; then
	report "crates/ contains crates but there is no root Cargo.toml"
else
	skip "Rust layer checks" "no root Cargo.toml yet"
fi

shopt -s nullglob
package_manifests=(packages/*/package.json)
shopt -u nullglob
if [[ ${#package_manifests[@]} -gt 0 ]]; then
	# Imports from spikes/ and interop/ are caught by noRestrictedImports in biome.jsonc; this
	# covers package.json.
	check_packages "${package_manifests[@]}"
	checked=1
else
	skip "package.json layer checks" "no packages/*/package.json yet"
fi
# interop/ may depend on packages/ and must not depend on spikes/ either.
if [[ -f interop/package.json ]]; then
	check_spike_deps interop/package.json
	checked=1
fi

if [[ ${errors} -gt 0 ]]; then
	fail "${errors} layer violation(s)"
fi
if [[ ${checked} -eq 1 ]]; then
	ran "Layer checks"
fi
