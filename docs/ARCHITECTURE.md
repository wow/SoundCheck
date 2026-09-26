# SoundCheck — architecture

Status: v0.1 architecture fixed 2026-09-24.

## Shape (v0.1)
```
src        React 19 + TS (Vite, Tailwind 4, shadcn, zustand, xstate)   -- renders; never holds PCM
src-tauri  Tauri 2 shell: commands, Channel<JobEvent>, read_peaks         -- thin; calls sc-engine
crates/sc-cli           headless binary: analyze | eval | cache | process | grid-check | bench | undo -- thin; calls sc-engine and only prints
crates/sc-engine        analyze (the per-file pipeline), run_batch(files, settings, cancel, on_event), cancellation, click player (cpal + rtrb)
crates/sc-analysis      loudness (ebur128 wrap + S-P95/S-top30/PLR + timeline), beats (beat-this, rten), grid solver (Huber LS, comb phase, kick-band anchor, octave order, thresholds, confidence, refit), DJ-safe report
crates/sc-dsp           gain, TPDF dither, primitives (biquad, kick-band filter, RMS/derivative onset), [v0.2 limiter, Re-Pitch], [v0.3 stretch]
crates/sc-io            decode (symphonia + opus, LAME delay/padding applied), iff (WAV/RF64/AIFF read+write, verbatim chunk carry), tagcopy (ID3v2/ID3v1/APEv2/Vorbis opaque carry + frame-level append), flac (flacenc + MD5 + SEEKTABLE + block carry), mp3gain (global_gain patch + CRC + undo), transaction (LengthPolicy, tiered verify, backup, journal, sidecar), rekordbox XML + CSV writers, cache, lofty read-only facade
crates/sc-core          AudioSpec/AudioBuffer, SampleIndex, Lufs/Lu/DbTp/DbFs/Bpm newtypes, Grid, reports, Plan, SkipReason, Error, IPC types (serde + ts-rs), feature "testsig" (synthetic signals)
```
Dependency direction: `sc-core <- sc-dsp <- sc-analysis`, `sc-core <- sc-io`, all `<- sc-engine <- {src-tauri, sc-cli}`. `sc-core` has no I/O. Not in v0.1: `sc-testkit`, SQLite, Chromaprint, pyramid files, `peaks://`, LAME, Rubber Band.

## Data flow per file
```
ANALYSE (streamed; cached)
  decode (delay/padding applied) -> f32 interleaved blocks
    -> ebur128 (M/S/I/LRA/TP; S sampled per 100 ms -> S-P95, S-top30, timeline, PLR)
    -> 22.05 kHz mono buffer (kept whole; freed after)
  beat-this (small model; bounded pool, one instance per worker) -> beats, downbeats, logits
  kick-band onset peaks (30-150 Hz, 1 ms) -> cached
  meter estimator (accent pattern at the finest pulse, templates incl. aksak, genre prior) -> Meter
  grid solver -> Grid { anchor, bpm, meter, first_downbeat_index, segments, residuals, verdict, confidence, alternatives }
  DJ-safe report, tag inventory (lofty read), cover thumbnail
  -> cache JSON (~/Library/Caches/app.soundcheck.desktop/analysis/<blake3(path)>.json, keyed by size+mtime+settings+version)

DECIDE (pure): AnalysisRecord + Settings{mode: DJ|Streaming, preset, ceiling, batch_mode: Prepare|Library, output} -> Plan { gain_db, short_by_lu, length_policy: Preserve|TrimHead{delta}, tags, xml, skip: Option<SkipReason> }

RENDER (streamed)
  lossless: decode -> [TrimHead] -> gain -> [TPDF if 16-bit] -> iff/flac writer with carried chunks/blocks -> tagcopy append
  mp3:      global_gain patch in place (no decode/encode) -> tagcopy append in padding
  transaction: preflight -> O_EXCL temp -> render -> fsync -> tiered verify -> backup + journal -> rename -> mtime -> sidecar
  batch artefacts: soundcheck-rekordbox.xml, grid-report.csv; per-file grid-check
```

## Key types (sc-core)
- `AudioSpec { sample_rate, channels }`, `AudioBuffer { spec, data: Vec<f32> }` interleaved; `SampleIndex(u64)`.
- `LoudnessReport { integrated, momentary_max, short_term_max, short_term_p95, short_term_top30, lra, true_peak, sample_peak, plr, dual_mono, timeline }`.
- `Meter { beats_per_bar, unit, grouping: Vec<u8> }` and `Grid { anchor, bpm, meter, first_downbeat_index, segments, residual_p95_ms, residual_max_ms, local_bpm_range, drift_ppm, verdict: Static|StaticWarn|Drifts, confidence: Green|Amber|Red, reasons, alternatives: { octave_up, octave_down, downbeat_shift } }`.
- `Plan`, `LengthPolicy`, `SkipReason` (`AlreadyAtTarget`, `WouldGetQuieter`, `UnsupportedFormat`, `UnsupportedChannels`, `DrmProtected`, `RekordboxUsbExport`, `SeratoTagsPresentInPlaceCut`, `GainFieldRange`, `Corrupt`, `Cancelled`), `JobEvent`, `IpcError`.

## Threading and IPC
- Analysis workers: a fixed set of threads (default half the logical cores, the performance cores on Apple silicon, at most 4; `--jobs` overrides), each with its own analyzer and beat model, taking files in order; each file runs decode -> loudness -> beats -> solver on one worker. Workers send events over a channel to the caller's thread, which forwards them: `Started`, progress at most every 100 ms, one terminal event per file (analysed, failed, cancelled), and a batch summary (done, total, ETA, x real time) at most every 500 ms. A failing file never stops the batch; a missing model stops it before any file. Records do not depend on the worker count. `analyze --no-grid` skips the model.
- Cancellation: one flag per batch, checked between decoded blocks, before every 30 s beat-model chunk and before the cache write, so a cancelled file leaves no cache entry.
- Memory per worker (M1, measured): loudness alone streams in about 12 MB; with the grid the model's inference dominates at about 450 MB peak footprint (resident size 600-800 MB while the allocator keeps freed pages), independent of track length because the model runs on 30 s chunks, and of its thread count. The pool size is chosen with this in mind.
- Model files are looked up, in order, in `SC_MODEL_DIR`, `models/` next to the executable, `../Resources/models` inside the app bundle, `models/` under the working directory (a checkout after `scripts/fetch-models.sh`) and `~/Library/Application Support/app.soundcheck.desktop/models`; a missing model stops the run before any file.
- Selected track: PCM held as i16 in memory for `read_peaks{file_id, level, from, to} -> ipc::Response` (64-sample level for the visible window) and for the click player producer (`Arc<Grid>` swapped atomically on edit; cpal callback only copies from an `rtrb` ring).
- `grid_refit{anchor?, bpm?, from_beat?}` is arithmetic over cached beats + onsets (< 5 ms) and returns residuals as raw f32 bytes.
- Tauri commands are async, return a `JobId`, and stream `JobEvent`s through `Channel` at <= 10 Hz per file.

## Storage
- Cache: central JSON per file (above). Sidecar `<file>.soundcheck.json` only for written outputs. Backups under `~/Music/SoundCheck Backups/<relative path>` with `journal.jsonl`. Settings via `tauri-plugin-store`.

## Verification tiers
WAV/AIFF: tee-hash of PCM as written + header/chunk table re-read. FLAC: full re-decode compared to the tee hash (+ `flac -t` in CI). MP3: frame re-walk, decode first/last 32 frames, `Track.delay/padding/num_frames` unchanged. Library mode additionally asserts identical frame count. Full re-measure (I/TP) via `sc-cli --verify=full` and nightly CI.

## Build & release
Cargo workspace (edition 2024, resolver 3), pnpm workspace, `scripts/verify.sh`, `ci.yml` (ubuntu + one macos-14 job), `release.yml` (tauri-action; signs + notarizes with the `APPLE_*` secrets from the first tag; forks without secrets get an unsigned "dev build" artifact), cargo-deny allowlist, cargo-about attribution.
