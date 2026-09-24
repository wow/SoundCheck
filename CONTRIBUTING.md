# Contributing to SoundCheck

Thanks for helping. SoundCheck is a desktop tool that touches people's music libraries, so the bar is: every change is tested, reviewed, signed off and traceable. This page is the contributor view; maintainers also read `docs/RELEASING.md`.

## Ground rules
- Licence: MIT OR Apache-2.0 for every contribution (see `LICENSE-MIT`, `LICENSE-APACHE`). No CLA. You certify the [Developer Certificate of Origin 1.1](https://developercertificate.org/) by signing off each commit (`git commit -s`).
- Dependencies must be MIT/Apache-2.0/MPL-2.0/BSD/ISC/Zlib/0BSD/CC0 (`deny.toml` enforces it). No copyleft engines, nothing that needs a paid licence.
- Be kind: `CODE_OF_CONDUCT.md` (Contributor Covenant) applies everywhere in the project.

## Set up
```
git clone https://github.com/wow/SoundCheck && cd SoundCheck
./scripts/setup-dev.sh        # installs the git hooks, enables sign-off, checks toolchains
./scripts/verify.sh           # fmt, clippy -D warnings, tests, typecheck, vitest
pnpm tauri dev
```
Toolchain: stable Rust (`rust-toolchain.toml`), Node 22+, pnpm 10+. macOS is the primary platform; Linux builds and runs the tests.

## Branches
- `main` is protected: pull requests only, CI green, linear history, signed commits. It is always releasable.
- Branch from `main` as `<type>/<short-slug>`: `feat/meter-estimator`, `fix/mp3-lame-delay`, `docs/release-checklist`, `perf/peaks-window`, `refactor/grid-solver`, `test/iff-proptest`, `chore/deps`, `ci/coverage`, `spike/…` (never merged; findings are written up in the issue or PR that closes the spike).
- Keep branches short-lived (one plan or one issue, ideally under a week). Rebase onto `main` rather than merging `main` in. Delete the branch after merge.
- Outside contributors work on a fork; maintainers may push branches to the main repository.

## Commits
Format: [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).
```
<type>(<scope>)!: <subject in the imperative, lowercase, no period, <= 72 chars>

<body: what changed and why, wrapped at 72; numbers with units>

Output-Change: yes            # only if the same input now yields different numbers, grids or bytes
BREAKING CHANGE: <what>       # only with the "!"
Closes #123
Signed-off-by: Your Name <you@example.com>
```
- Types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`, `revert`.
- Scopes: `core`, `io`, `dsp`, `analysis`, `engine`, `cli`, `desktop` (Tauri shell), `ui`, `ipc`, `docs`, `ci`, `deps`, `release`. Several scopes: `feat(io,cli): …`.
- `!` / `BREAKING CHANGE:` means a persisted schema or contract changed: sidecar JSON, analysis cache, settings store, `sc-cli --json` output, rekordbox XML we write, IPC types.
- `Output-Change: yes` means a user who re-runs the app on the same file gets a different loudness value, grid, or written bytes. Such a PR regenerates goldens (`UPDATE_GOLDEN=1 cargo test -p <crate>`, diff reviewed as numbers) and can only ship in a minor release.
- One logical change per commit; tests in the same commit; the tree builds and passes at every commit you push.
- Reverts: `revert: <original subject>` with `This reverts commit <sha>.` in the body.
- Sign-off is required on every commit (`-s`). Maintainers additionally sign commits and tags cryptographically. AI-assisted commits keep their `Co-Authored-By:` trailer; the human who signs off is responsible for the change.
- The local `commit-msg` hook (installed by `scripts/setup-dev.sh`) rejects a bad subject or a missing sign-off; CI checks PR titles and sign-offs again.

## Pull requests
- One PR = one logical change; the PR title is the future squash-commit subject and must follow the commit format.
- Fill the template: summary, user-visible behaviour, tests run (paste the `./scripts/verify.sh` summary), whether it is an output change, docs touched.
- Size: aim for under ~400 changed lines excluding fixtures, goldens and generated bindings; split otherwise or say why.
- Required checks: `fmt`, `clippy`, `test`, `deny`, `typecheck`, `vitest`, `dco`, `pr-title`, and `macos-build` on PRs. A PR that adds a dependency states its licence in the description.
- Review: a maintainer reviews every PR; DSP, grid, file-layer and IPC changes get a numerical review (tolerances, determinism, byte-preservation). Address comments with new commits; the squash keeps history clean.
- Merge: squash only, by a maintainer, after CI is green. The squash message is the PR title plus the PR description.

## Tests
Every change ships with its tests, in the same commit:
- Unit tests next to the code (`#[cfg(test)]`, `*.test.ts`), under 100 ms each, deterministic: no wall clock, no audio device, no network, no files outside the repo.
- DSP and analysis changes add a golden or tolerance test (`crates/<name>/tests/golden_*.rs` against `tests/fixtures/golden/`) that states its tolerance and why, plus a criterion bench for hot loops. Goldens regenerate only with `UPDATE_GOLDEN=1 cargo test -p <crate>` and the numeric diff is reviewed in the PR.
- File-layer changes extend the fixture matrix (per-block SHA-256 of every carried chunk, frame and block) and the `proptest` cases for the IFF walker, tag-block copier and MP3 frame walker.
- Loudness changes pass the EBU Tech 3341/3342 cases (`SC_EBU_TESTSET=1`, fetched by `scripts/fetch-ebu-testset.sh`); grid changes pass the synthetic click-track suite (4/4, 3/4, 6/8, 9/8, 7/8).
- CLI changes add an `assert_cmd` test; engine changes test event order, cancellation and skip reasons with a null audio sink; UI changes use Vitest + Testing Library with mocked IPC.
- Slow tests are `#[ignore]` behind an opt-in environment variable and run nightly. Never loosen a tolerance or add `#[ignore]` to get green. Bug fixes add a regression test named after the issue.

## Issues and security
- Use the issue templates (bug with `sc-cli analyze --json` output; grid mismatch with `sc-cli grid-check` output, DJ-app version and screenshot; loudness mismatch; feature request).
- Security problems: do not open an issue; follow `SECURITY.md` (private report, fix within 7 days, GitHub advisory).
