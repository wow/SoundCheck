#!/usr/bin/env bash
# Fails when Rust unit tests are written inline. Unit tests live in their own file next to the
# module they test (`foo.rs` -> `foo/tests.rs`, `mod.rs` or `lib.rs` -> `tests.rs`), declared
# with `#[cfg(test)] mod tests;`, so source files hold only code; integration tests live in each
# crate's `tests/` folder.
set -euo pipefail
cd "$(dirname "$0")/.."
found=$(git ls-files --cached --others --exclude-standard -- '*.rs' |
  xargs grep -nE '^[[:space:]]*mod tests[[:space:]]*\{' 2>/dev/null || true)
if [ -n "$found" ]; then
  echo "inline unit tests; move them to a tests.rs next to the module:"
  echo "$found"
  exit 1
fi
