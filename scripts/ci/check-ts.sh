#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

# The TypeScript projects of the Bun workspace in the root package.json: the packages, the
# interop runner that drives them against other implementations, and the demo's applications.
shopt -s nullglob
manifests=(packages/*/package.json)
shopt -u nullglob
if [[ -f interop/package.json ]]; then
	manifests+=(interop/package.json)
fi
if [[ -f demo/package.json ]]; then
	manifests+=(demo/package.json)
fi

# WHY per directory, and before anything else: a directory under packages/ without a package.json
# is in no workspace, so nothing below would lint, type-check or test it, and the job would
# still be green. This must not depend on whether interop/ exists.
if [[ -d packages ]]; then
	stray_files="$(find packages -maxdepth 1 -type f -print -quit)"
	if [[ -n "${stray_files}" ]]; then
		fail "packages/ holds files outside a package, so nothing checks them: ${stray_files}"
	fi
	shopt -s nullglob
	package_dirs=(packages/*/)
	shopt -u nullglob
	for package_dir in "${package_dirs[@]}"; do
		if [[ ! -f "${package_dir}package.json" ]]; then
			fail "${package_dir} has no package.json, so nothing would check the files in it"
		fi
	done
fi

if [[ ${#manifests[@]} -eq 0 ]]; then
	skip "TypeScript checks" "no packages/*/package.json yet"
	exit 0
fi

# WHY: a nested config can switch off any rule for its package (Biome with "root": false,
# oxlint by nesting), which would make the root rules optional.
nested_configs="$(find packages interop demo \( -name 'biome.json' -o -name 'biome.jsonc' -o -name '.oxlintrc.json' -o -name '.oxlintrc.jsonc' -o -name 'oxlint.config.*' \) -not -path '*/node_modules/*' 2>/dev/null || true)"
if [[ -n "${nested_configs}" ]]; then
	fail "lint configuration belongs in the repository root only; remove: ${nested_configs//$'\n'/, }"
fi

# WHY: with one lockfile per project, interop's lockfile embedded the devDependencies of the
# package it depends on, and a change to one made --frozen-lockfile fail in the other.
nested_locks="$(find packages interop demo \( -name 'bun.lock' -o -name 'bun.lockb' \) -not -path '*/node_modules/*' 2>/dev/null || true)"
if [[ -n "${nested_locks}" ]]; then
	fail "the workspace has one bun.lock in the repository root; remove: ${nested_locks//$'\n'/, }"
fi
if [[ ! -f package.json ]]; then
	fail "there is no root package.json, so the projects are not one Bun workspace"
fi

# WHY no path argument: biome.jsonc decides what is covered (and why spikes/ and interop/peers/ are not).
biome ci --error-on-warnings
# WHY a second linter: Biome 2.5 has no rule that bans `as` assertions. See .oxlintrc.jsonc.
oxlint --deny-warnings packages interop demo

bun install --frozen-lockfile

# WHY into a variable first: under pipefail, `bun pm ls | grep -q` fails when grep exits at the
# first match and bun gets SIGPIPE, although the match was found.
workspace_members="$(bun pm ls)"

for manifest in "${manifests[@]}"; do
	dir="$(dirname "${manifest}")"
	# WHY: a project left out of "workspaces" gets no dependencies from the root install, and
	# then fails below for a reason that is hard to read.
	if ! grep -Fq "$(jq -r '.name' "${manifest}")@workspace:${dir}" <<<"${workspace_members}"; then
		fail "${manifest} is not a member of the workspace in the root package.json"
	fi
	# WHY required instead of skipped when absent: a package without a typecheck script would
	# pass this job without ever being type-checked.
	if [[ "$(jq -r '.scripts.typecheck // empty' "${manifest}")" == "" ]]; then
		fail "${manifest} has no \"typecheck\" script"
	fi
	(
		cd "${dir}"
		bun run typecheck
		# WHY not for interop and demo: they have no unit tests. What interop checks is the run
		# against other implementations, and the demo is checked by running it (demo/check.ts); both
		# need a relay and a browser, which are not part of this job.
		if [[ "${dir}" == packages/* ]]; then
			bun test
		fi
	)
done
ran "TypeScript checks (biome, oxlint, typecheck, bun test) for ${#manifests[@]} project(s)"
