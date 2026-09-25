# SoundCheck — product definition

Status: v0.1 scope fixed 2026-09-24. Open source. Source of truth for scope and UX.

## One sentence
SoundCheck levels a DJ library to one loudness and makes new tracks start exactly on the grid, so rekordbox, Serato and Traktor agree with it, without touching the sound or losing a single tag.

## Who it is for
1. **DJs** preparing new tracks and USB exports (rekordbox/CDJ, Serato, Traktor, Engine DJ) and levelling libraries they already cue-pointed.
2. **Producers and edit makers** who bounce at inconsistent levels and want a track that drops into rekordbox or Ableton at bar 1 with an exact BPM.
3. Streaming/podcast users (secondary): integrated-loudness presets.

## The three promises (and their tests)
1. **Same loudness** — gain only; when the ceiling would be hit the row says "Short by X LU". Test: EBU Tech 3341/3342 conformance; within 0.1 LU of ffmpeg `ebur128`.
2. **Fits the grid in rekordbox** — after export, rekordbox 7's own fresh analysis puts beat 1 at 0.000 s (+/-2 ms) with the same 2-decimal BPM on >= 18/20 tracks of the owner's labelled set; wrong tracks are fixed in the app in < 30 s.
3. **Nothing lost** — every tag block and the cover art carried byte-for-byte (proven by per-block SHA-256 fixtures and shown on screen), originals backed up, Revert available, filenames unchanged.

## What v0.1 does
1. **Analyse** WAV, AIFF, FLAC, MP3 (M4A/AAC/ALAC, Ogg/Opus analyse-only): loudness (I, max-M, max-S, **S-P95**, S-top30, LRA, true peak, PLR, timeline), beats/downbeats/BPM (0.01) with a three-state confidence and reason chips, **meter with grouping** (4/4, 3/4, 6/8, 9/8 aksak 2+2+2+3, 5/8, 7/8, 10/8), static-vs-drifts verdict with residual statistics, spec (rate/depth/channels/codec, DJ-safe flag), tags and cover art (read).
2. **Loudness fix** behind a top-level **DJ | Streaming** switch. DJ: one fixed, persisted **DJ target** on S-P95 (default -11 until measured; "Calibrate from my library" sets it once) and **Club hot** -8. Streaming: Spotify -14, Apple -16 on Integrated. Gain only, ceiling-capped (-0.5 dBTP DJ, -1.0 streaming), "Short by X LU" when headroom runs out; MP3 via lossless `global_gain` in 1.5 dB steps with the residual shown. Per-track override.
3. **Grid inspect and fix** per track: waveform at beat zoom auto-located to bar 1, bar ruler, grid overlay, residual lane (per bar / per beat), drift card with numbers; fixes: drag anchor (kick-snapped), nudge +/-1 ms (10 ms, 1 beat), type/tap BPM, x2 / /2, `1`-`n` set which beat is beat 1, meter picker (runner-up first), re-fit from here, reset, undo; ruler and click accents follow the grouping; **click audition** (accented downbeat) on a minimal in-app player.
4. **Two modes**: **Prepare** (new tracks; lossless files cut at the head to the first downbeat with at most one beat of pre-roll; in place allowed; MP3/AAC get a metadata-only grid + "Convert to AIFF copy + cut") and **Library** (never change length; loudness only by default; rekordbox XML grid opt-in; cut-to-grid only into a new folder). Auto-Library when Serato tags are present.
5. **Export**: in place with backup (default) or to a folder; format source / AIFF / WAV 24-bit / FLAC; DJ-safe validation (44.1/48 kHz, integer PCM, stereo, plain PCM header, FORM AIFF); tags `TBPM` + `TXXX:BPM`, ReplayGain 2.0, `SOUNDCHECK` record; **rekordbox XML per batch** (carrier, with Analysis Lock advice); `grid-report.csv`; output self-check (`grid-check`); rekordbox USB exports refused with explanation; FLAC re-analysis notice; stale Serato Autotags notice.
6. **Trust surface**: Action column (what will happen, in dB/LU/ms) separate from Status (where it is); "Done, verified" only after tiered verification; on-screen tag diff ("Verified: 27/27 identical, 3 added") with the cover rendered from the output file; Revert; three-line skip/error copy; positions-shift badge on Prepare rows.
7. **Batch UX**: drop folders/files, analyse-only, Needs-review queue (`N`/`Enter`/`Esc`), filter chips, progress and cancel, resume from cache, per-batch CSV.
8. **`sc-cli`** with `analyze <files> --json` (exactly the table's fields, one document per file; `--no-grid` for loudness only, `--bpm-range 70-180` for the DJ app's range, `--no-cache`), `cache path|clear` (the analysis cache lives under `~/Library/Caches/app.soundcheck.desktop/analysis`, keyed by file size and time, settings and app version), `process`, `grid-check`, `bench`, `undo`. A file that fails prints an error document and the exit code is 2.

## Not in v0.1 (with the version that may bring it back)
True-peak limiter, Re-Pitch, meters with ballistics, A/B audition, device picker, Serato BeatGrid GEOB (v0.2). Easy warp, AAC/M4A writes, Opus gain, SQLite cache, Chromaprint (v0.3). Key detection, pre-flight checks, per-platform penalty, Windows, updater, light theme (v0.4). Never: pitch correction, EQ, enhancement, copyleft or paid engines, filename suffixes, direct ANLZ/Engine DB writes.

## UX principles
1. Transparent by construction: the default path changes only gain; anything else is labelled with numbers.
2. Show, then act: analysis first, a table of what would happen, then Process.
3. One control per concern; correct defaults; advanced knobs exist but are quiet.
4. Never lose data: originals, metadata, journal, sidecar, Revert.
5. Fast enough to feel instant: >= 20x real time per core with the small model; cached re-runs in seconds.
6. Keyboard-first for the review loop (`N`, `Enter`, `1`-`4`, arrows, `Space`), mouse for everything else.

## Open source
MIT OR Apache-2.0. No pricing, licence keys or trial logic. Every shipped dependency is permissive (MIT/Apache-2.0/MPL-2.0/BSD/ISC/Zlib). Public repo from the first commit; every tagged build signed and notarized (the owner holds an Apple Developer ID).

## Definition of done for v0.1
1. Clean M1 Mac: `git clone` -> `pnpm tauri dev` in < 15 min following the README; CI green on `ubuntu-latest` and `macos-14`.
2. 50 mixed files (WAV/AIFF/FLAC/MP3) analysed in < 3 min on an 8-core M1 with the small model (>= 20x real time per core).
3. EBU Tech 3341/3342 cases within tolerance under `SC_EBU_TESTSET=1`; integrated loudness within 0.1 LU of ffmpeg `ebur128` on 20 files.
4. 20-track evaluation set: BPM within +/-0.02 on >= 18/20 after at most one x2 / /2; after export, rekordbox 7 fresh analysis shows beat 1 as the first grid entry within 15 ms of the start, same phase and BPM, on >= 18/20 (hand-run release gate; octave flips count as failures unless the XML carrier pins them).
5. 12-file fixture set round-trips with byte-identical carried blocks for ID3v2.3/2.4 (APIC + GEOB), FLAC, AIFF, WAV; asserted in `cargo test`; crash-injection matrix passes.
6. MP3 `global_gain` output has the identical frame count and LAME header; a cue at sample N decodes to the same sample; undo restores the original hash.
7. Three DJs who are not the maintainer complete drop -> inspect -> fix one anchor -> export -> rekordbox without help.
8. GitHub Release `v0.1.0`: signed + notarized DMG with the small model, CHANGELOG, licence files, `cargo deny check` green.
