#!/usr/bin/env bash
# Prepare a release: bump the single repository version, generate the changelog section, commit on a
# release branch and open the pull request. The maintainer tags after the merge. See docs/RELEASING.md.
# Usage: scripts/release.sh <X.Y.Z[-alpha.N|-beta.N|-rc.N]> [--dry-run]
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="${1:-}"; DRY=0; [ "${2:-}" = "--dry-run" ] && DRY=1
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?$ ]] || { echo "usage: scripts/release.sh <X.Y.Z[-alpha.N|-beta.N|-rc.N]> [--dry-run]" >&2; exit 2; }
for t in git git-cliff cargo pnpm jq perl; do command -v "$t" >/dev/null 2>&1 || { echo "missing tool: $t" >&2; exit 2; }; done
run() { if [ "$DRY" = 1 ]; then printf 'dry-run: %s\n' "$*"; else "$@"; fi; }
fail() { printf 'release: %s\n' "$1" >&2; exit 1; }

# 1. Preconditions: on main, clean, at origin/main, version files consistent.
[ "$(git branch --show-current)" = "main" ] || fail "run from main"
[ -z "$(git status --porcelain)" ] || fail "working tree is not clean"
git fetch -q origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || fail "main is not at origin/main (pull --ff-only first)"
CURRENT="$(perl -0777 -ne 'print $1 if /\[workspace\.package\][^\[]*?\nversion\s*=\s*"([^"]+)"/' Cargo.toml)"
[ -n "$CURRENT" ] || fail "no [workspace.package] version in Cargo.toml"
PKG="$(jq -r .version package.json)"
[ "$PKG" = "$CURRENT" ] || fail "version drift: Cargo.toml $CURRENT vs package.json $PKG"
if jq -e 'has("version")' src-tauri/tauri.conf.json >/dev/null 2>&1; then fail "tauri.conf.json must not carry a version key (it inherits the workspace version)"; fi
grep -Eq '^version(\.workspace)?[[:space:]]*=[[:space:]]*(\{[[:space:]]*workspace[[:space:]]*=[[:space:]]*true|true)' src-tauri/Cargo.toml || fail "src-tauri/Cargo.toml must use version.workspace = true"
git tag -l "v$VERSION" | grep -q . && fail "tag v$VERSION already exists"
SUGGEST="$(git cliff --bumped-version 2>/dev/null || true)"
echo "current $CURRENT -> requested $VERSION (git-cliff suggests ${SUGGEST:-n/a})"
if [ -n "$SUGGEST" ] && [ "$SUGGEST" != "v$VERSION" ] && [[ "$VERSION" != *-* ]]; then
  echo "warning: git-cliff derives $SUGGEST from the commits; continuing with $VERSION (docs/RELEASING.md section 1 explains when they differ)" >&2
fi

# 2. Bump the two version files and refresh lockfiles.
BRANCH="chore/release-v$VERSION"
run git switch -c "$BRANCH"
if [ "$DRY" = 0 ]; then
  perl -0777 -pi -e 's/(\[workspace\.package\][^\[]*?\nversion\s*=\s*")[^"]+(")/${1}'"$VERSION"'${2}/' Cargo.toml
  jq --indent 2 --arg v "$VERSION" '.version = $v' package.json > package.json.tmp && mv package.json.tmp package.json
  cargo update --workspace --offline 2>/dev/null || cargo update --workspace
  pnpm install --lockfile-only >/dev/null
else
  echo "dry-run: bump Cargo.toml + package.json to $VERSION; cargo update --workspace; pnpm install --lockfile-only"
fi

# 3. Changelog section from the commits (Keep a Changelog groups via cliff.toml).
[ -f CHANGELOG.md ] || printf '# Changelog\n\nAll notable changes to SoundCheck are listed here. The format follows Keep a Changelog; versions follow SemVer (docs/RELEASING.md).\n\n' > CHANGELOG.md
run git cliff --unreleased --tag "v$VERSION" --prepend CHANGELOG.md
SECTION="$(git cliff --unreleased --tag "v$VERSION" --strip all 2>/dev/null || true)"

# 4. Verification and smoke build.
run ./scripts/verify.sh
run cargo build --release -p sc-cli
if [ "$DRY" = 0 ]; then
  cargo run -q --release -p sc-cli -- --version | grep -q "$VERSION" || fail "sc-cli --version does not print $VERSION"
fi

# 5. Commit (DCO sign-off), push, open the PR.
run git add Cargo.toml Cargo.lock package.json pnpm-lock.yaml CHANGELOG.md
run git commit -s -m "chore(release): v$VERSION"
run git push -u origin "$BRANCH"
if command -v gh >/dev/null 2>&1; then
  BODY="$(printf 'Release v%s.\n\nChecklist: docs/release-checklist.md ticked on the candidate commit.\n\n%s\n' "$VERSION" "$SECTION")"
  run gh pr create --title "chore(release): v$VERSION" --body "$BODY"
else
  echo "gh is not installed: open the PR for $BRANCH by hand with title 'chore(release): v$VERSION'"
fi

cat <<MSG

Next (maintainer, after the squash merge):
  git switch main && git pull --ff-only
  git tag -s v$VERSION -m "SoundCheck v$VERSION"
  git push origin v$VERSION
release.yml then builds, signs, notarizes and publishes; do the same-day post-release check (docs/RELEASING.md section 3, step 6).
MSG
