#!/usr/bin/env bash
# SoundCheck verification. Prints one PASS/FAIL/SKIP line per step; logs in target/verify/.
# Exit 1 if any step failed (CI-friendly). Steps for parts of the workspace that do not exist yet are SKIPped.
set -u
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOGDIR="$ROOT/target/verify"; mkdir -p "$LOGDIR"
cd "$ROOT"
FAILED=0
run_step() { # name, condition(0=run), command...
  local name="$1" cond="$2"; shift 2
  if [ "$cond" != "0" ]; then printf 'SKIP  %-22s (not present)\n' "$name"; return; fi
  local t0 t1; t0=$(date +%s)
  if "$@" >"$LOGDIR/$name.log" 2>&1; then
    t1=$(date +%s); printf 'PASS  %-22s %3ss\n' "$name" "$((t1-t0))"
  else
    t1=$(date +%s); printf 'FAIL  %-22s %3ss  -> %s\n' "$name" "$((t1-t0))" "$LOGDIR/$name.log"
    grep -E "^(error|warning: unused|test .* FAILED|failures:|---- .* stdout ----|FAIL |✗|Error:)" "$LOGDIR/$name.log" | head -12 | sed 's/^/      /'
    FAILED=1
  fi
}
HAS_CARGO=1; [ -f Cargo.toml ] && HAS_CARGO=0
HAS_FE=1;    [ -f package.json ] && HAS_FE=0
HAS_FE_DEPS=1; [ -d node_modules ] && HAS_FE_DEPS=0

run_step cargo-fmt     "$HAS_CARGO" cargo fmt --all -- --check
run_step cargo-clippy  "$HAS_CARGO" cargo clippy --workspace --all-targets -- -D warnings
run_step cargo-test    "$HAS_CARGO" cargo test --workspace
if [ "$HAS_FE" = "0" ] && [ "$HAS_FE_DEPS" != "0" ]; then echo "SKIP  frontend                (run: pnpm install)"; else
  run_step fe-typecheck  "$HAS_FE" pnpm typecheck
  run_step fe-lint       "$HAS_FE" pnpm lint
  run_step fe-test       "$HAS_FE" pnpm test -- --run
fi
if [ "$FAILED" = "0" ]; then echo "RESULT PASS"; else echo "RESULT FAIL"; fi
exit $FAILED
