#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=scripts/ci/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "${REPO_ROOT}"

actionlint
shellcheck --external-sources scripts/ci/*.sh scripts/agent-demo.sh demo/broadcast/scripts/*.sh
ran "Workflow and script checks (actionlint, shellcheck)"
