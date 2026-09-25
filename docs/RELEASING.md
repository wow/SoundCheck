# Releasing SoundCheck (maintainers)

Contributors read `CONTRIBUTING.md`; this page is the maintainer's procedure for versions, tags, builds and keys.

## 1. Versioning policy
- **SemVer 2.0.0, one version for the whole repository.** Source of truth: `[workspace.package].version` in the root `Cargo.toml`. `package.json` mirrors it (CI job `version-sync` fails if they differ). `tauri.conf.json` has no `version` key, so the bundle takes the workspace version through `src-tauri/Cargo.toml` (`version.workspace = true`). All crates are `publish = false`.
- **Before 1.0** (`0.MINOR.PATCH`): MINOR = a feature milestone, a breaking change or an output change. PATCH = fixes, performance and docs only; a patch never changes results for an unchanged input.
- **Pre-releases**: `0.1.0-alpha.N` (maintainer-only), `0.1.0-beta.N` (external testers), `0.1.0-rc.N` (only fixes after an rc). The pre-release suffix is in the tag and in the two version files. If the macOS bundler ever refuses a suffix, keep the suffix in the tag only and note it here.
- **1.0.0** is declared when: the planned v0.x feature set has shipped; the sidecar, cache, settings and CLI JSON schemas have not changed for two consecutive minors; a Windows build exists; no open P1. After 1.0: MAJOR for schema or contract breaks, MINOR for features and output changes, PATCH for fixes.
- **Schemas**: every persisted JSON (`<file>.soundcheck.json`, analysis cache, settings store, `sc-cli --json`) carries `"schema": <int>`. Readers accept older schemas and migrate forward; they refuse a newer schema with a clear error. A schema bump is `feat!:`.
- **Output changes** (`Output-Change: yes` footer): new goldens in the same PR (`UPDATE_GOLDEN=1 cargo test -p <crate>`, diff reviewed as numbers), a line under "Output changes" in the changelog, minor release only. The analysis cache is keyed by app version and model version, so old results are never mixed with new ones.
- **Model weights**: the bundled `beat-this` model has its own version string recorded in the cache key and sidecar; swapping weights is an output change.
- **Build metadata**: `sc-cli --version` and the About panel print `0.1.0 (a1b2c3d, 2026-10-01)`; the git SHA and date never go into the tag.
- **Dependencies**: caret ranges, lockfiles committed, Dependabot weekly grouped PRs for cargo, npm and GitHub Actions; actions pinned by SHA; `cargo deny` (licences + advisories) on every PR.
- **Platforms**: macOS 12+; `aarch64` DMG for `0.0.x` and `-alpha` tags, `universal` DMG from the first `-beta` onward. Windows arrives with v0.4 (MSI needs numeric versions; pre-release suffixes will be tag-only there).

## 2. Cadence
- Minor releases when a feature milestone is complete and `docs/release-checklist.md` is green.
- Patch releases within a week of any merged user-facing fix; security fixes within 7 days of the report (`SECURITY.md`).
- Never on a Friday afternoon; a release needs the post-release check the same day.

## 3. Release steps
1. **Readiness on `main`**: `./scripts/verify.sh` green on the candidate commit; `docs/release-checklist.md` hand-run gates ticked (all sections for `v0.x.0` and `-rc`; only "Files" and "App" for a patch).
2. **`scripts/release.sh <version>`** (the only place versions are bumped): confirms the version against `git cliff --bumped-version`, edits the two version files, refreshes lockfiles, prepends the generated section to `CHANGELOG.md`, commits `chore(release): v<version>` with sign-off on branch `chore/release-v<version>`, and opens the PR. Polish wording in the changelog in that PR (user-facing, numbers with units), never restructure it.
3. **Merge** the release PR (squash) once CI is green.
4. **Tag** (human, signed, on the merge commit):
   ```
   git switch main && git pull --ff-only
   git tag -s v0.1.0 -m "SoundCheck v0.1.0"
   git push origin v0.1.0
   ```
5. **`release.yml`** (trigger `v*`): asserts tag == workspace version; runs the verification; builds the DMG and the `sc-cli` tarball; signs, notarizes (`notarytool`) and staples; writes `SHA256SUMS.txt`; regenerates `THIRD_PARTY.md`; generates release notes with `git cliff --latest --strip header`; creates the GitHub Release **as a draft**, attaches every asset, and publishes it in the last step (pre-release when the tag contains `-`). The `sc-cli` binary is signed and notarized too; a bare binary cannot carry a stapled ticket, so Gatekeeper fetches it online on first run. **If a run fails before the publish step**, the release is still a draft: fix the workflow on `main` and re-run it for the same tag with `gh workflow run release.yml --ref main -f tag=vX.Y.Z`; the draft and its assets are reused. Published releases are immutable in this repository: assets cannot be added or changed afterwards, and a tag whose immutable release existed can never be reused, even after deleting the release. A failed run therefore costs a tag; fix the pipeline and release the next patch version. Assets: `SoundCheck_<version>_<arch>.dmg`, `sc-cli-<version>-<target>.tar.gz`, `SHA256SUMS.txt`, `THIRD_PARTY.md`; from v0.2 also `latest.json` + `.sig` (minisign) for the updater, final releases only.
6. **Post-release check, same day**: download the DMG on a clean macOS user account; `spctl -a -vv -t open --context context:primary-signature SoundCheck_<version>_<arch>.dmg` and, after mounting, `spctl -a -vv -t exec SoundCheck.app` both say "accepted, source=Notarized Developer ID"; launch; drop three files, analyse, export one, open it in rekordbox. Log the result in the release PR.
7. **Announce**: the GitHub Release is the announcement; README badge updates itself.

## 4. Hotfixes and maintenance branches
- Default: fix on `main` via a normal PR, then release a patch from `main`.
- If `main` already carries unreleased minor features: `git switch -c release/0.1 v0.1.0`, `git cherry-pick -x <sha>` of the fix commits (they land on `main` first), bump the patch version with `scripts/release.sh 0.1.1` on that branch, tag from it. Delete `release/0.1` after `v0.2.0` ships.
- Never rewrite `main`, never move or delete a tag, never delete a GitHub Release. A broken release is superseded by the next patch; edit its notes with a "Known issue: use v0.1.1" line.

## 5. Keys and secrets
| Asset | Where | Notes |
|---|---|---|
| Apple Developer ID Application certificate (`.p12`) + password | GitHub Actions secrets `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` in environment `release` | environment requires the maintainer's approval before secrets are exposed; never in the repo, never on a shared machine |
| App Store Connect API key | `APPLE_API_ISSUER` (issuer id), `APPLE_API_KEY` (key id), `APPLE_API_KEY_CONTENT` (the `.p8` file contents) in the same environment | `release.yml` writes the key to a file for `notarytool`; scoped to the Developer role |
| Maintainer commit/tag signing key | the maintainer's machine (SSH signing) and GitHub "signing keys" | `git config gpg.format ssh`, `user.signingkey`, `commit.gpgsign true`, `tag.gpgSign true` |
| Updater key (v0.2, minisign) | private key in the maintainer's password manager and secret `TAURI_SIGNING_PRIVATE_KEY`; public key committed in `tauri.conf.json` | losing it strands every installed copy; back it up before the first updater release |

Rotation: revoke the certificate in the Apple developer portal, issue a new one, replace the three secrets, re-run the latest release workflow with `workflow_dispatch`; previously notarized builds stay valid. A leaked signing key is revoked the same day and the event is written in `SECURITY.md`'s history section.

## 6. Repository settings (set once when the repository is created)
- Ruleset on `main`: require pull request; required checks `fmt`, `clippy`, `test`, `deny`, `typecheck`, `vitest`, `dco`, `pr-title`, `macos-build`, `version-sync`; require linear history; require signed commits; block force pushes and deletions; no bypass list.
- Merge settings: squash only; default squash message "pull request title and description"; auto-delete head branches.
- Tag ruleset on `v*`: only maintainers may create; no updates, no deletions. Enable "immutable releases" if the repository setting is available.
- Environment `release` with the maintainer as required reviewer; secrets live only there. Deployment branches: tags `v*` and `main` (the latter only for `workflow_dispatch` re-runs of an existing tag).
- Dependabot: `.github/dependabot.yml` for `cargo`, `npm`, `github-actions`, weekly, grouped.
- Security: private vulnerability reporting on; `SECURITY.md` with the 7-day promise.

## 7. Changelog
`CHANGELOG.md` follows Keep a Changelog and is generated by `git-cliff` (`cliff.toml`) from the commits, one section per tag, newest first. Group mapping: `feat` -> Added; `fix` -> Fixed; `perf`, `refactor` -> Changed; `!`/`BREAKING CHANGE` -> Breaking; `Output-Change: yes` -> Output changes; `fix(security)` or a `Security:` footer -> Security; `docs`, `test`, `ci`, `build`, `chore` are omitted unless user-facing. The release notes on GitHub are the same section.
