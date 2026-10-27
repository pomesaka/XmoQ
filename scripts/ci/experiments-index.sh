#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

# Writes the index at the end of docs/experiments/README.md: every record, under the Phase of the
# roadmap that its "Roadmap item" names first, by date. With --check, fails where the README is
# not what this would write, or a record lacks the "Date", "Commit SHA" or "Roadmap item" line.
#
# WHY generated: a record is added with nearly every change, and an index kept by hand misses
# them (57 of 214 records were linked from nowhere when this was written). WHY by the Phase its
# own line names: the record says what it was for, and nothing here has to judge it.
#
# The Phases are the "## Phase <N> — <title>" headings of docs/roadmap.md. A Phase keeps its number
# and its heading after its work is done, and a new one is added at the end: the records name the
# Phase they were for by its number.

readme=docs/experiments/README.md
marker="<!-- The index below is written by scripts/ci/experiments-index.sh (mise run fmt). -->"

# One line a record: the Phase it names (or "none"), then its line in the index.
records() {
	local record phase name heading
	for record in docs/experiments/2026-*.md; do
		# WHY grep -o and not sed: a sed pattern with ".*" before "Phase" takes the last one the line
		# names, where a record that names two is filed under the first.
		phase="$(grep -m 1 '^- Roadmap item:' "${record}" | grep -oE 'Phase [0-9][0-9a-z]*' | head -n 1 | cut -d ' ' -f 2 || true)"
		name="$(basename "${record}")"
		heading="$(sed -n '1s/^# //p' "${record}")"
		printf '%s\t- %s [%s](%s)\n' "${phase:-none}" "${name:0:10}" "${heading}" "${name}"
	done
}

index() {
	local all phase title lines
	all="$(records)"
	echo "${marker}"
	echo
	echo "## Index"
	# The Phases in the roadmap's order, then the records that name none, or a Phase the roadmap
	# does not have.
	while IFS=$'\t' read -r phase title; do
		lines="$(awk -F '\t' -v phase="${phase}" '$1 == phase { print $2 }' <<<"${all}")"
		all="$(awk -F '\t' -v phase="${phase}" '$1 != phase' <<<"${all}")"
		[[ -n "${lines}" ]] || continue
		printf '\n### Phase %s — %s\n\n%s\n' "${phase}" "${title}" "${lines}"
	done < <(sed -n 's/^## Phase \([0-9a-z]*\) — \(.*\)$/\1\t\2/p' docs/roadmap.md)
	lines="$(awk -F '\t' '{ print $2 }' <<<"${all}")"
	if [[ -n "${lines}" ]]; then
		printf '\n### Not tied to a Phase\n\n%s\n' "${lines}"
	fi
}

# The README above the marker, as it is.
before="$(awk -v marker="${marker}" '$0 == marker { exit } { print }' "${readme}")"
written="${before}"$'\n\n'"$(index)"

# The records that lack one of the lines at the top of the template, as "record: line" lines.
# WHY checked: the index is built from "Roadmap item", and a record without it is filed under no
# Phase without anyone noticing; "Date" and "Commit SHA" say what tree a run was made on.
lacking() {
	local record head
	for record in docs/experiments/2026-*.md; do
		for head in "Date" "Commit SHA" "Roadmap item"; do
			if ! grep -q "^- ${head}:" "${record}"; then
				echo "${record}: - ${head}:"
			fi
		done
	done
}

if [[ "${1:-}" == --check ]]; then
	missing="$(lacking)"
	if [[ -n "${missing}" ]]; then
		fail "records without a line of the template's top (${readme}):"$'\n'"${missing}"
	fi
	if [[ "${written}" != "$(cat "${readme}")" ]]; then
		fail "${readme}: its index is not what scripts/ci/experiments-index.sh writes; run mise run fmt"
	fi
	ran "The index of docs/experiments/"
else
	printf '%s\n' "${written}" >"${readme}"
fi
