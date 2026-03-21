#!/usr/bin/env bash
set -euo pipefail

# Exit early if not in a git repo
git rev-parse --git-dir >/dev/null 2>&1 || exit 0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$REPO_ROOT/scripts/lib/common.sh"
configure_commit_git_identity_if_needed 2>/dev/null || true
