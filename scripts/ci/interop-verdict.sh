#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

# Last step of the interop job. interop.sh records "ran" after every pairing ended as expected,
# and interop-scope.sh records "skipped" when it tells the job to leave the pairings out.
#
# WHY: the steps of the job are conditional. If the condition is ever wrong for all of them,
# nothing runs, nothing fails, and the job is green. This step is not conditional.

verdict_file="${RUNNER_TEMP:?this script is a step of the interop job}/interop-verdict"
if [[ ! -f "${verdict_file}" ]]; then
	fail "the job neither ran the pairings nor reported that it skipped them"
fi
verdict="$(<"${verdict_file}")"
case "${verdict}" in
ran | skipped) echo "Interop job: ${verdict}" ;;
*) fail "unknown verdict \"${verdict}\" in ${verdict_file}" ;;
esac
