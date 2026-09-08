#!/usr/bin/env bash
# Copy the CI workflow definitions from docs/ci/workflows into .github/workflows.
#
# They are kept outside .github/ because the integration token that pushes Bluey's
# pull requests lacks the `workflow` scope; a maintainer runs this once and commits
# the result (see docs/ci/README.md).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/docs/ci/workflows"
DST="$ROOT/.github/workflows"

mkdir -p "$DST"
for file in "$SRC"/*.yml; do
  cp "$file" "$DST/$(basename "$file")"
  echo "→ installed .github/workflows/$(basename "$file")"
done
echo "✓ review the files, then: git add .github/workflows && git commit -m 'ci: add workflows'"
