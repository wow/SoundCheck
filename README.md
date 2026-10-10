# SoundCheck

**Level a DJ library to one loudness and make every track start exactly on the grid, without touching the sound or losing a single tag.**

SoundCheck is a free, open-source desktop app for DJs and producers. Drop a folder, see what every track needs, fix the few grids that are wrong, and export files that rekordbox, Serato and Traktor agree with.

> **Status: in development, no usable release yet.** What works today, from source:
> - **The app** analyses a dropped library (loudness, BPM, meter and bar 1, with a Needs-review queue) and opens any track in a grid view to inspect and fix its grid while a click plays along, with live IN/OUT meters, an original/processed A/B and a volume control. It does not write files yet.
> - **`sc-cli apply`** already changes WAV, AIFF and FLAC files: gain and an optional head trim (snapped back up to 1 ms to the quietest frame and faded in over 2 ms), every other chunk, block and tag carried byte for byte, verified, with a backup in `~/Music/SoundCheck Backups` and `sc-cli undo`.
> - **`sc-cli process <files> --batch-mode prepare|library`** exports: the planned gain, the Prepare cut and the tags written in place after a backup (or as copies with `--out`), verified against the planned length, with a sidecar recording the exported grid; grid edits and confirmations made in the app carry over to the exported file.
> - **`sc-cli process`** also writes the batch's **rekordbox XML** (`soundcheck-rekordbox.xml`: each track's location and a 4/4 `TEMPO` with bar 1 where the export put it) and a **`grid-report.csv`**, into the `--out` folder or `~/Music/SoundCheck/exports/<date time>/`; **`sc-cli xml <files>`** writes the XML for files already exported or only analysed.
> - **`sc-cli plan --batch-mode prepare|library`** previews what an export will do to each file: the gain, the head cut (`Cut 0.21 s`, `Starts on bar 1`, `Starts on a bar line (bar 1 at 8.00 s)`, `Not cut: bar 1 1.00 s in`, or `Not cut: grid needs review` until the grid is confirmed), the tags, or why a file is left to the rekordbox XML (MP3/AAC for now) or skipped.
>
> Exporting from the app, the exported-grid self-check and MP3 output come next. The first pre-release, `v0.1.0-alpha.1`, follows once exported files pass rekordbox 7's own analysis; `v0.1.0` is the first release for everyone. Watch the releases page or the changelog.

## What v0.1 will do

- **Same loudness**: every track lands on one persisted DJ target (measured on the loud parts of the track, not the intro) or on a streaming target (integrated loudness). Gain only. When the true-peak ceiling would be hit, the row says "Short by X LU" instead of squashing the sound.
- **Fits the grid**: beats, downbeats and an exact two-decimal BPM, with a static grid you can inspect and fix in seconds (anchor, nudge, BPM, half/double, which beat is beat 1). Odd meters such as 9/8 and 6/8 are recognised and shown with their grouping. For tracks whose tempo changes, the grid can be fitted to the start. In **Prepare** mode (new tracks) lossless files are cut so bar 1 starts a few milliseconds after the start of the file, removing at most one beat (a file whose bar 1 lies further in is not cut); in **Library** mode (tracks already in a DJ app, with cue points) a file's length never changes. A per-batch rekordbox XML carries the grid.
- **Nothing lost**: every ID3, Vorbis, RIFF and AIFF block, cover art and DJ-app cue blob is carried byte for byte and verified after writing. Originals are backed up and every change can be reverted. Files are never renamed.
- Formats: WAV, AIFF, FLAC and MP3 in and out (MP3 loudness through the lossless global-gain patch, no re-encode); M4A, AAC, ALAC, Ogg and Opus are analysed only.
- A headless `sc-cli` that prints exactly the numbers the app shows.

## What v0.1 does not do

No limiter or compression, no EQ, no "enhancement", no pitch correction, no warping or time-stretch, no key detection, no Windows or Linux build, no auto-updater, no writes to M4A/AAC/Opus, no direct writes into rekordbox or Engine databases, no filename suffixes. Some of these are planned for later versions; the changelog says which.

## Building from source

Requires a stable Rust toolchain, Node 22+ and pnpm 11+. On macOS, Xcode command line tools.

```
git clone https://github.com/wow/SoundCheck && cd SoundCheck
./scripts/setup-dev.sh        # git hooks, sign-off, toolchain check
pnpm install
./scripts/fetch-models.sh     # beat-tracking model files (11 MB, checksum-verified) into models/
./scripts/verify.sh           # fmt, clippy, tests, typecheck, vitest
pnpm tauri dev                # run the app
pnpm dev:mock                 # the UI alone in a browser, with synthetic tracks (open /?demo=1)
cargo run --release -p sc-cli -- analyze <file>          # loudness, BPM, meter and bar 1
cargo run --release -p sc-cli -- plan <files>            # what processing would do to each file
cargo run --release -p sc-cli -- plan <files> --batch-mode prepare   # ... and what exporting would write
cargo run --release -p sc-cli -- process <files> --batch-mode prepare   # export (backup first; --out <dir> for copies)
cargo run --release -p sc-cli -- apply <files> --gain-db -3   # change files (backup first; --out <dir> for copies)
cargo run --release -p sc-cli -- undo <files>            # put the previous version back
cargo run --release -p sc-cli -- journal                 # recorded changes; `recover` finishes interrupted ones
cargo run --release -p sc-cli -- bench <file>            # speed of each analysis stage
```

Unsigned development builds show a Gatekeeper warning on first launch; release builds are signed and notarized. To run a development build you downloaded, remove the quarantine flag: `xattr -dr com.apple.quarantine SoundCheck.app`.

## Contributing

See `CONTRIBUTING.md` for branches, commits (Conventional Commits with a DCO sign-off), tests and the review process, and `docs/RELEASING.md` for how versions and releases work. Product scope lives in `docs/PRODUCT.md`, the architecture in `docs/ARCHITECTURE.md`.

## Licence

MIT OR Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`). Every dependency in the shipped binary is under a permissive licence; the full list is generated into `THIRD_PARTY.md` for each release.
