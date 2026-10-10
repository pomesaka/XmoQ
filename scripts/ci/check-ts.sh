#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

shopt -s nullglob
manifests=(packages/*/package.json)
shopt -u nullglob

if [[ ${#manifests[@]} -eq 0 ]]; then
	if [[ -d packages ]] && [[ -n "$(find packages -type f -not -path '*/node_modules/*' -print -quit)" ]]; then
		fail "packages/ contains files but no packages/*/package.json, so nothing would check them"
	fi
	skip "TypeScript checks" "no packages/*/package.json yet"
	exit 0
fi

# WHY: a nested config can switch off any rule for its package (Biome with "root": false,
# oxlint by nesting), which would make the root rules optional.
nested_configs="$(find packages \( -name 'biome.json' -o -name 'biome.jsonc' -o -name '.oxlintrc.json' -o -name '.oxlintrc.jsonc' -o -name 'oxlint.config.*' \) -not -path '*/node_modules/*')"
if [[ -n "${nested_configs}" ]]; then
	fail "lint configuration belongs in the repository root only; remove: ${nested_configs//$'\n'/, }"
fi

# WHY no path argument: biome.jsonc decides what is covered (and why spikes/ is not).
biome ci --error-on-warnings
# WHY a second linter: Biome 2.5 has no rule that bans `as` assertions. See .oxlintrc.jsonc.
oxlint --deny-warnings packages

for manifest in "${manifests[@]}"; do
	dir="$(dirname "${manifest}")"
	# WHY required instead of skipped when absent: a package without a typecheck script would
	# pass this job without ever being type-checked.
	if [[ "$(jq -r '.scripts.typecheck // empty' "${manifest}")" == "" ]]; then
		fail "${manifest} has no \"typecheck\" script"
	fi
	(
		cd "${dir}"
		bun install --frozen-lockfile
		bun run typecheck
		bun test
	)
done
ran "TypeScript checks (biome, oxlint, typecheck, bun test) for ${#manifests[@]} package(s)"
