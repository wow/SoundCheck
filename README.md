# SoundCheck

**Level a DJ library to one loudness and make every track start exactly on the grid, without touching the sound or losing a single tag.**

SoundCheck is a free, open-source desktop app for DJs and producers. Drop a folder, see what every track needs, fix the few grids that are wrong, and export files that rekordbox, Serato and Traktor agree with.

> **Status: under construction.** This repository holds the project skeleton. The first usable release is `v0.1.0`; nothing here processes audio yet. Watch the releases page or the changelog.

## What v0.1 will do

- **Same loudness**: every track lands on one persisted DJ target (measured on the loud parts of the track, not the intro) or on a streaming target (integrated loudness). Gain only. When the true-peak ceiling would be hit, the row says "Short by X LU" instead of squashing the sound.
- **Fits the grid**: beats, downbeats and an exact two-decimal BPM, with a static grid you can inspect and fix in seconds (anchor, nudge, BPM, half/double, which beat is beat 1). Odd meters such as 9/8 and 6/8 are recognised and shown with their grouping. Files are cut so beat 1 is the first sample, and a per-batch rekordbox XML carries the grid.
- **Nothing lost**: every ID3, Vorbis, RIFF and AIFF block, cover art and DJ-app cue blob is carried byte for byte and verified after writing. Originals are backed up and every change can be reverted. Files are never renamed.
- Formats: WAV, AIFF, FLAC and MP3 in and out (MP3 loudness through the lossless global-gain patch, no re-encode); M4A, AAC, ALAC, Ogg and Opus are analysed only.
- A headless `sc-cli` that prints exactly the numbers the app shows.

## What v0.1 does not do

No limiter or compression, no EQ, no "enhancement", no pitch correction, no warping or time-stretch, no key detection, no Windows or Linux build, no auto-updater, no writes to M4A/AAC/Opus, no direct writes into rekordbox or Engine databases, no filename suffixes. Some of these are planned for later versions; the changelog says which.

## Building from source

Requires a stable Rust toolchain, Node 22+ and pnpm 10+. On macOS, Xcode command line tools.

```
git clone https://github.com/wow/SoundCheck && cd SoundCheck
./scripts/setup-dev.sh        # git hooks, sign-off, toolchain check
pnpm install
./scripts/fetch-models.sh     # beat-tracking model files (11 MB, checksum-verified) into models/
./scripts/verify.sh           # fmt, clippy, tests, typecheck, vitest
pnpm tauri dev                # run the app
pnpm dev:mock                 # the UI alone in a browser, with synthetic tracks (open /?demo=1)
cargo run --release -p sc-cli -- plan <files>            # what processing would do to each file
cargo run --release -p sc-cli -- analyze <file>          # loudness, BPM, meter and bar 1
cargo run --release -p sc-cli -- bench <file>            # speed of each analysis stage
```

Unsigned development builds show a Gatekeeper warning on first launch; release builds are signed and notarized. To run a development build you downloaded, remove the quarantine flag: `xattr -dr com.apple.quarantine SoundCheck.app`.

## Contributing

See `CONTRIBUTING.md` for branches, commits (Conventional Commits with a DCO sign-off), tests and the review process, and `docs/RELEASING.md` for how versions and releases work. Product scope lives in `docs/PRODUCT.md`, the architecture in `docs/ARCHITECTURE.md`.

## Licence

MIT OR Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`). Every dependency in the shipped binary is under a permissive licence; the full list is generated into `THIRD_PARTY.md` for each release.
