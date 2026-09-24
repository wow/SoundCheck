# Release checklist (hand-run gates)

Run before tagging any `v0.x.0` or `-rc` (a patch release needs only the Files and App sections). Tick in the release PR. Procedure and versioning policy: `docs/RELEASING.md`.

## Loudness
- [ ] `SC_EBU_TESTSET=1 cargo test --workspace -- --ignored ebu` passes.
- [ ] `scripts/cross-check-ffmpeg.sh` on the 20-file corpus: |I_ours - I_ffmpeg| <= 0.1 LU on every file.

## Grid: the 20-track rekordbox protocol
Owner-owned set (outside the repo), labelled once in rekordbox: BPM and first-downbeat time. Mix: house, techno, DnB, hip-hop; >= 5 MP3; >= 3 with Serato tags.
- [ ] `sc-cli analyze --json` on the set: BPM within +/-0.02 of the label on >= 18/20 after at most one x2 / /2.
- [ ] Prepare-mode export of the lossless tracks; `sc-cli grid-check` on every output: BPM +/-0.005, anchor 0 +/-5 ms.
- [ ] rekordbox 7, Track Analysis Mode Normal with the owner's BPM range (70-180), fresh import of the exported files (not XML): beat 1 is the first grid entry, within 15 ms of the start, same phase, BPM equal to the written value on >= 18/20. Record every miss with the file, rekordbox's BPM and first-beat time. Repeat one file with Cloud Analysis off.
- [ ] XML path: import the batch XML (tracks, not only the playlist); confirm `Inizio`/`Bpm` match; enable Analysis Lock; export to USB; read `PQTZ` with pyrekordbox; record whether the grid survived.
- [ ] Serato DJ Pro (Set Beatgrid/BPM on) and Traktor (Automatic range) on 5 tracks each: BPM equal, bar 1 on the downbeat; record cue drift on the Serato-tagged files (must be 0).

## Files
- [ ] `cargo test -p sc-io` fixture matrix green (per-block SHA-256, headers, `flac -t`, ffprobe, LAME delay, MP3 undo).
- [ ] Crash-injection matrix green (`SC_TEST_CRASH_AFTER_STEP=1..6`).
- [ ] Manual: process one Serato-tagged MP3 in Library mode, reopen in Serato: cues and grid unchanged, gain re-levelled per the Autotags notice.

## App
- [ ] `./scripts/verify.sh` green; `cargo deny check` green; `THIRD_PARTY.md` regenerated.
- [ ] Signed + notarized DMG opens on a clean macOS 14 and 15 machine with no dialog (or README `xattr` section present for dev builds).
- [ ] CHANGELOG entry; README "does not do" list current.
