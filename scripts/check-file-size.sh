#!/usr/bin/env bash
# Fails when a source file is longer than the cap (default 700 lines), counting every line:
# inline tests, comments and blank lines included. Generated IPC bindings are exempt. A file
# over the cap is split by concern (a module folder, a tests file, subcomponents), not trimmed.
# Usage: scripts/check-file-size.sh [max-lines]
set -euo pipefail
MAX="${1:-700}"
cd "$(dirname "$0")/.."
status=0
while IFS= read -r file; do
  [ -f "$file" ] || continue
  lines=$(wc -l < "$file" | tr -d ' ')
  if [ "$lines" -gt "$MAX" ]; then
    echo "$file: $lines lines, more than $MAX; split it by concern"
    status=1
  fi
done < <(git ls-files --cached --others --exclude-standard -- \
  '*.rs' '*.ts' '*.tsx' '*.js' '*.sh' '*.py' ':!src/lib/ipc/generated/**')
exit $status
