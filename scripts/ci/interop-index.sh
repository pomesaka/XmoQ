#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

# Writes the list of files at the end of interop/README.md: each script of interop/ with the
# first sentence of the comment it begins with, the drivers that matrix.ts starts apart from
# the parts they share. With --check, fails where the README is not what this would write.
#
# WHY generated: what a run has to show is written at the top of its driver, and the README
# that told it again missed drivers that matrix.ts runs.

readme=interop/README.md
marker="<!-- The list below is written by scripts/ci/interop-index.sh (mise run fmt). -->"

# The first sentence of the comment a file begins with.
summary() {
	awk '
		/^\/\/ ?/ { sub(/^\/\/ ?/, ""); if ($0 == "") exit; text = text (text == "" ? "" : " ") $0; next }
		{ exit }
		END { print text }
	' "$1" | sed -e 's/^Interop driver: //' -e 's/\. .*/./' | awk '{ print toupper(substr($0, 1, 1)) substr($0, 2) }'
}

rows() {
	local driver="$1" file name
	for file in interop/*.ts; do
		name="$(basename "${file}")"
		if grep -qF "\"${name}\"" interop/matrix.ts; then
			[[ "${driver}" == yes ]] || continue
		else
			[[ "${driver}" == no ]] || continue
		fi
		printf '| [%s](%s) | %s |\n' "\`${name}\`" "${name}" "$(summary "${file}")"
	done
}

index() {
	echo "${marker}"
	echo
	echo "## Files"
	echo
	echo "The drivers that \`matrix.ts\` starts. How to start one alone, and what each of its runs has to show, is at the top of the file."
	echo
	echo "| Driver | What it runs |"
	echo "|---|---|"
	rows yes
	echo
	echo "The parts they share."
	echo
	echo "| File | What it is |"
	echo "|---|---|"
	rows no
}

# The README above the marker, as it is.
before="$(awk -v marker="${marker}" '$0 == marker { exit } { print }' "${readme}")"
written="${before}"$'\n\n'"$(index)"

if [[ "${1:-}" == --check ]]; then
	if [[ "${written}" != "$(cat "${readme}")" ]]; then
		fail "${readme}: its list of files is not what scripts/ci/interop-index.sh writes; run mise run fmt"
	fi
	ran "The list of files of interop/"
else
	printf '%s\n' "${written}" >"${readme}"
fi
