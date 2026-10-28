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

# Crates that must stay free of others, as "package name:what it stays free of:banned crates".
# WHY cc, cmake and bindgen for a C compiler: a crate that builds C calls one of them from its
# build script (ring 0.17.14, libsqlite3-sys 0.38.2, whisper-rs-sys 0.15.0 and openh264-sys2 0.9.8
# of Cargo.lock all do). The tracks are read by programs that are to build without one, and
# xmoq-viewer does no input or output and has no need of one (it takes xmoq-room-token without
# its feature "keys", which needs ring).
# WHY libsqlite3-sys for SQLite: rusqlite reaches SQLite through it.
FREE_OF=(
	"xmoq-chat-tracks:C compiler:cc cmake bindgen"
	"xmoq-transcript-tracks:C compiler:cc cmake bindgen"
	"xmoq-viewer:C compiler:cc cmake bindgen"
	"xmoq-meeting-tracks:C compiler:cc cmake bindgen"
	"xmoq-pet-tracks:C compiler:cc cmake bindgen"
	"xmoq-watch:SQLite:libsqlite3-sys"
)

# A program that is there for interop/ alone; no crate of the workspace depends on it.
INTEROP_ONLY_CRATES=(xmoq-quiet-peer)

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

	check_crate_layers "${metadata}"

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

	local free_of
	for entry in "${FREE_OF[@]}"; do
		name="${entry%%:*}"
		free_of="${entry#*:}"
		free_of="${free_of%%:*}"
		if ! jq -e --arg name "${name}" '.packages[] | select(.name == $name)' <<<"${metadata}" >/dev/null; then
			report "the workspace has no package named ${name}; update FREE_OF in this script"
			continue
		fi
		# WHY normal and build edges, with every feature: as for the I/O-free crates above.
		tree="$(cargo tree --package "${name}" --edges normal,build --all-features --prefix none --format '{p}' --locked | awk '{print $1}' | sort -u)"
		before=${errors}
		for banned in ${entry##*:}; do
			if grep -Fxq "${banned}" <<<"${tree}"; then
				report "${name} must need no ${free_of} but depends on ${banned}; see the path with 'cargo tree --package ${name} --edges normal,build --all-features --invert ${banned}'"
			fi
		done
		if [[ ${errors} -eq ${before} ]]; then
			ran "${name} needs no ${free_of}"
		fi
	done

	local dependents
	for name in "${INTEROP_ONLY_CRATES[@]}"; do
		# WHY every edge, dev ones too: a test of a crate that reaches the program builds it as well.
		dependents="$(cargo tree --workspace --invert "${name}" --edges all --prefix none --format '{p}' --locked | awk '{print $1}' | sort -u | grep -Fxv "${name}" || true)"
		if [[ -n "${dependents}" ]]; then
			report "${name} is for interop/ alone, and these depend on it: ${dependents//$'\n'/, }"
		else
			ran "No crate depends on ${name}"
		fi
	done
}

# The layer of a crate of the workspace, a number; empty for a name that is none of them. This is
# the one table of the crates' layers.
crate_layer() {
	case "$1" in
	xmoq-moqt | xmoq-loc | xmoq-msf | xmoq-again) echo 0 ;;
	xmoq-moqt-io | xmoq-room-token | xmoq-chat-tracks | xmoq-transcript-tracks | xmoq-meeting-tracks | xmoq-pet-tracks | xmoq-picture | xmoq-speech) echo 1 ;;
	xmoq-publisher | xmoq-subscriber | xmoq-relay-core | xmoq-viewer) echo 2 ;;
	xmoq-viewer-io) echo 3 ;;
	# The programs: nothing of the workspace depends on them.
	xmoq-relay | xmoq-watch | xmoq-agent | xmoq-chat | xmoq-garden | xmoq-pet | xmoq-transcriber | xmoq-moqt-client | xmoq-quiet-peer) echo 4 ;;
	*) echo "" ;;
	esac
}

# The crates are layers: one depends on the crates of a lower layer alone. WHY normal and build
# dependencies and not dev ones: a test may use a crate above (a program's test harness), which
# the crate itself does not build on.
check_crate_layers() {
	local metadata="$1" name layer dep dep_layer before=${errors}
	while IFS= read -r name; do
		layer="$(crate_layer "${name}")"
		if [[ -z "${layer}" ]]; then
			report "the crate ${name} has no layer; add it to crate_layer in this script"
			continue
		fi
		while IFS= read -r dep; do
			dep_layer="$(crate_layer "${dep}")"
			# Empty: a crate from outside the workspace.
			if [[ -n "${dep_layer}" ]] && ((dep_layer >= layer)); then
				report "${name} (layer ${layer}) depends on ${dep} (layer ${dep_layer}); a crate depends on crates of a lower layer alone (crate_layer in this script)"
			fi
		done < <(jq -r --arg name "${name}" '.packages[] | select(.name == $name) | .dependencies[] | select(.kind != "dev") | .name' <<<"${metadata}")
	done < <(jq -r '.packages[].name' <<<"${metadata}")
	if [[ ${errors} -eq ${before} ]]; then
		ran "The layers of the Rust crates"
	fi
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

# The layer of a package under packages/, a number; empty for a name that is none of them.
package_layer() {
	case "$1" in
	# The reader of JSON is one that the packages of the tracks share. The client of a room
	# is asked over HTTP and needs none of the others.
	@xmoq/moqt | @xmoq/loc | @xmoq/json | @xmoq/room-client) echo 0 ;;
	# What an endpoint does with tracks, whatever they carry, built on the transport alone.
	@xmoq/tracks) echo 1 ;;
	# Each publishes or follows its tracks by @xmoq/tracks: the catalog, the chat's tracks,
	# a transcript's and the pet's.
	@xmoq/msf | @xmoq/chat | @xmoq/transcript | @xmoq/pet) echo 2 ;;
	# Media publishes its catalog by @xmoq/msf.
	@xmoq/media) echo 3 ;;
	@xmoq/meeting) echo 4 ;;
	# The room is run, not built on: nothing depends on it.
	@xmoq/room) echo 5 ;;
	*) echo "" ;;
	esac
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
	# The packages are layers: one may depend on the packages of a lower layer, and on none of
	# its own or a higher one. MoQ Transport and LOC, the format of a media Object, know nothing
	# of each other; the streaming format is built on the transport; what encodes and plays
	# media is built on all of them.
	# WHY a package that is not listed fails: a new one would otherwise depend on anything.
	# WHY the value of a dependency is read as well as its name: a package is reached under any
	# name by "file:../moqt", "workspace:packages/moqt" or "npm:@xmoq/moqt@*".
	local name layer dependency other other_name other_directory other_layer
	for manifest in "$@"; do
		name="$(jq -r '.name' "${manifest}")"
		layer="$(package_layer "${name}")"
		if [[ -z "${layer}" ]]; then
			report "${manifest}: ${name} has no layer; add it to package_layer in scripts/ci/check-layers.sh"
			continue
		fi
		while IFS= read -r dependency; do
			for other in "$@"; do
				[[ "${other}" != "${manifest}" ]] || continue
				other_name="$(jq -r '.name' "${other}")"
				other_directory="$(basename "$(dirname "${other}")")"
				# The name, as the key or within the value, or the directory at the end of a path.
				if ! grep -Eq "(^|[:@ ])${other_name}([:@ ]|\$)|(^|[:/ ])(\.\./|packages/)${other_directory}(/|\$)" <<<"${dependency}"; then
					continue
				fi
				other_layer="$(package_layer "${other_name}")"
				# One without a layer is reported for itself.
				[[ -n "${other_layer}" ]] || continue
				if [[ "${other_layer}" -ge "${layer}" ]]; then
					report "${manifest}: ${name} (layer ${layer}) depends on ${other_name} (layer ${other_layer}), which is not a layer below it: ${dependency}"
				fi
			done
		done < <(package_deps "${manifest}")
	done
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
