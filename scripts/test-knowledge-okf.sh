#!/usr/bin/env bash
# Verify the canonical knowledge bundle's OKF v0.2 structure and links, and
# that scripts/check_okf.py rejects the violations it is meant to catch.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECKER="$PROJECT_ROOT/scripts/check_okf.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

write_bundle() {
  local bundle="$1"

  mkdir -p "$bundle/apis"
  cat > "$bundle/index.md" <<'EOT'
---
okf_version: "0.2"
---
# Knowledge

- [APIs](apis/)
- [Update Log](log.md)
EOT
  cat > "$bundle/log.md" <<'EOT'
# Log

## 2026-10-10

* Created.
EOT
  cat > "$bundle/apis/index.md" <<'EOT'
# APIs

- [Widgets](widgets.md)
EOT
  cat > "$bundle/apis/widgets.md" <<'EOT'
---
type: Specification
title: "Widgets"
description: "Fixture concept."
---
# Widgets

See [the index](index.md).
EOT
}

assert_rejected() {
  local bundle="$1"
  local output="$TMP_DIR/check-output.txt"
  shift

  if python3 "$CHECKER" "$bundle" >"$output" 2>&1; then
    echo "expected checker to reject $bundle" >&2
    return 1
  fi
  for expected in "$@"; do
    if ! grep -Fq "$expected" "$output"; then
      echo "missing expected error for $bundle: $expected" >&2
      cat "$output" >&2
      return 1
    fi
  done
}

python3 "$CHECKER" "$PROJECT_ROOT/knowledge"

valid="$TMP_DIR/valid/knowledge"
write_bundle "$valid"
python3 "$CHECKER" "$valid"

missing_type="$TMP_DIR/missing-type/knowledge"
write_bundle "$missing_type"
sed -i.bak '/^type:/d' "$missing_type/apis/widgets.md" && rm "$missing_type/apis/widgets.md.bak"
assert_rejected "$missing_type" "apis/widgets.md: frontmatter must contain a non-empty 'type'"

unlisted="$TMP_DIR/unlisted/knowledge"
write_bundle "$unlisted"
cp "$unlisted/apis/widgets.md" "$unlisted/apis/gadgets.md"
assert_rejected "$unlisted" "apis/gadgets.md: not listed in apis/index.md"

broken_link="$TMP_DIR/broken-link/knowledge"
write_bundle "$broken_link"
echo "See [missing](missing.md)." >> "$broken_link/apis/widgets.md"
assert_rejected "$broken_link" "apis/widgets.md: link target does not exist: missing.md"

bad_log="$TMP_DIR/bad-log/knowledge"
write_bundle "$bad_log"
echo "## Yesterday" >> "$bad_log/log.md"
assert_rejected "$bad_log" "invalid log heading"

root_specs="$TMP_DIR/root-specs/knowledge"
write_bundle "$root_specs"
mkdir -p "$TMP_DIR/root-specs/specs"
echo "# Demo" > "$TMP_DIR/root-specs/specs/demo.md"
assert_rejected "$root_specs" "specs/demo.md: specs belong in knowledge/, not in specs/"

echo "test-knowledge-okf: all checks passed"
