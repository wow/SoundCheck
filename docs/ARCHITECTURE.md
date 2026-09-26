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
ADD (on drop): collect_audio_files (folders walked in natural order; dot-files (one leading dot, incl. AppleDouble `._*`) and symlinked folders skipped, names starting with "..." kept;
  NFC duplicates dropped) -> probe in parallel (lofty headers and tags, cover art not read: codec incl. ALAC vs AAC, rate, depth,
  float WAV, bitrate, duration, title/artist/album, first DJ-unsafe reason) -> FileEntry rows before any decoding

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

DECIDE (pure, about 28 ns per row): decide(AnalysisRecord, Codec, DecideSettings{mode: DJ|Streaming, target, ceiling, bpm_range})
  -> Plan { measured, gain: Gain{gain_db, short_by_lu, true_peak_after} | GlobalGain{steps, gain_db, residual_lu} (MP3, 1.5051 dB steps) | AtTarget,
            skip: AnalyseOnly{codec} | Silent, review: [Confidence | Drifts | OutsideBpmRange | TagBpmDisagrees{tag} | NoGrid] (a grid that fits with elevated residuals is marked "check" in the BPM column, not queued), status }
  turning down is always allowed; a boost stops at the true-peak ceiling and the rest is "short by"; export adds batch_mode, length policy, tags and XML

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
- `Plan`, `GainPlan`, `ReviewReason`, `DecideSettings`, `Codec` (sc-core `plan`), `LengthPolicy`, `SkipReason` (`AnalyseOnly`, `Silent`; with export: `WouldGetQuieter`, `UnsupportedFormat`, `UnsupportedChannels`, `DrmProtected`, `RekordboxUsbExport`, `SeratoTagsPresentInPlaceCut`, `GainFieldRange`, `Corrupt`, `Cancelled`), `JobEvent`, `IpcError`.

## Threading and IPC
- Analysis workers: a fixed set of threads (default a quarter of the logical cores, at most 4; `--jobs` overrides; the beat model already spreads each file over every core, so on an 8-core M1 two workers are the fastest, 54x real time against 40x for one, and each worker adds about 440 MB), each with its own analyzer and beat model, taking files in order; each file runs decode -> loudness -> beats -> solver on one worker. Workers send events over a channel to the caller's thread, which forwards them: `Started`, progress at most every 100 ms, one terminal event per file (analysed, failed, cancelled), and a batch summary (done, total, ETA, x real time) at most every 500 ms. A failing file never stops the batch; a missing model stops it before any file. Records do not depend on the worker count. `analyze --no-grid` skips the model.
- Cancellation: one flag per batch, checked between decoded blocks, before every 30 s beat-model chunk and before the cache write, so a cancelled file leaves no cache entry.
- Memory per worker (M1, measured): loudness alone streams in about 12 MB; with the grid the model's inference dominates at about 450 MB peak footprint (resident size 600-800 MB while the allocator keeps freed pages), independent of track length because the model runs on 30 s chunks, and of its thread count. The pool size is chosen with this in mind.
- Model files are looked up in `SC_MODEL_DIR` alone when it is set, otherwise, in order, in `models/` next to the executable, `../Resources/models` inside the app bundle, `models/` under the working directory (a checkout after `scripts/fetch-models.sh`), in debug builds the checkout's own `models/`, and `~/Library/Application Support/app.soundcheck.desktop/models`; a missing model stops the run before any file. The app bundle carries the small model in `Contents/Resources/models`: `src-tauri/tauri.bundle.conf.json` adds it as a resource for release and CI bundles, so a plain `cargo build` needs no model files.
- Selected track: PCM held as i16 in memory for `read_peaks{file_id, level, from, to} -> ipc::Response` (64-sample level for the visible window) and for the click player producer (`Arc<Grid>` swapped atomically on edit; cpal callback only copies from an `rtrb` ring).
- `grid_refit{anchor?, bpm?, from_beat?}` is arithmetic over cached beats + onsets (< 5 ms) and returns residuals as raw f32 bytes.
- Tauri commands (bodies in `src-tauri/src/shell.rs`, plain Rust over `sc_engine::Session`): `expand_paths(paths) -> FileEntry[]` (walk and probe, on a blocking thread), `analyze({fileIds, analysis}, onEvent: Channel<JobEvent>) -> JobId` (returns at once; the job runs `run_job` on its own thread), `cancel_job(jobId)`, `set_decide_settings(settings) -> Replan{revision, plans}` (refused with `invalidArgument` outside the limits: target -30..-4 LUFS, ceiling -6..0 dBTP, BPM range 40..300), `calibration_target() -> Lufs?`, `restore_session() -> SessionSnapshot` (the rows the engine holds, for a window that reloads; running jobs are cancelled), `clear_session()` (Clear list: cancels running jobs and forgets the files; ids are never reused), `app_version`. Every command is async; the heavier ones run on a blocking thread. Each `analysed` event carries the settings revision its plan was decided with, so the UI keeps the newest plan when a replan and an analysis cross. Events per job: `started`, `progress` (at most every 100 ms per file), one of `analysed` (row + plan, under 2 KB) / `failed` / `cancelled`, `batch` (at most every 500 ms), `aborted` when the job cannot start, `finished` last. Plugins: dialog (open files and folders), store (settings), opener.

## Storage
- Cache: central JSON per file (above). Sidecar `<file>.soundcheck.json` only for written outputs. Backups under `~/Music/SoundCheck Backups/<relative path>` with `journal.jsonl`. Settings (`settings.json`) and the track list's file paths (`library.json`, re-added on launch and served from the cache) via `tauri-plugin-store`, each with a `schema` field.

## Verification tiers
WAV/AIFF: tee-hash of PCM as written + header/chunk table re-read. FLAC: full re-decode compared to the tee hash (+ `flac -t` in CI). MP3: frame re-walk, decode first/last 32 frames, `Track.delay/padding/num_frames` unchanged. Library mode additionally asserts identical frame count. Full re-measure (I/TP) via `sc-cli --verify=full` and nightly CI.

## Build & release
Cargo workspace (edition 2024, resolver 3), pnpm workspace, `scripts/verify.sh`, `ci.yml` (ubuntu + one macos-14 job), `release.yml` (tauri-action; signs + notarizes with the `APPLE_*` secrets from the first tag; forks without secrets get an unsigned "dev build" artifact), cargo-deny allowlist, cargo-about attribution.
