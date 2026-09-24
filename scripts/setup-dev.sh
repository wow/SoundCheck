#!/usr/bin/env bash
# One-time developer setup: git hooks, sign-off, toolchain check. Safe to re-run.
set -euo pipefail
cd "$(dirname "$0")/.."

git config core.hooksPath scripts/git-hooks
git config format.signoff true
git config pull.rebase true
git config merge.ff only
echo "git: hooks -> scripts/git-hooks, format.signoff=true, pull.rebase=true"

if [ "$(git config --get commit.gpgsign || true)" != "true" ]; then
  cat <<'MSG'
note: commit signing is not enabled. Maintainers must sign commits and tags; contributors are encouraged to:
  git config gpg.format ssh
  git config user.signingkey ~/.ssh/id_ed25519.pub
  git config commit.gpgsign true
  git config tag.gpgSign true
  (add the same public key as a *signing* key in GitHub > Settings > SSH and GPG keys)
MSG
fi

ok=1
for tool in cargo rustc pnpm node git-cliff; do
  if command -v "$tool" >/dev/null 2>&1; then
    printf '%-10s %s\n' "$tool" "$("$tool" --version 2>/dev/null | head -1)"
  else
    printf '%-10s MISSING\n' "$tool"; [ "$tool" = git-cliff ] || ok=0
  fi
done
command -v git-cliff >/dev/null 2>&1 || echo "git-cliff is only needed for /release: cargo install git-cliff  (or brew install git-cliff)"
[ "$ok" = 1 ] || { echo "install the missing tools, then re-run"; exit 1; }
echo "ready: ./scripts/verify.sh"
