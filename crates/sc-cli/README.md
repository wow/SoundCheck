# sc-cli

The headless SoundCheck binary. `sc-cli analyze <file> --json` prints exactly the fields the desktop table shows; `plan` says what processing would do to each file (with `--batch-mode prepare|library [--grid-only] [--lead-ms N] [--no-xml]`, also what exporting would do: gain, head cut (planned from where the render makes it, up to 1 ms before the cut asked; `Not cut: grid needs review` for a row whose grid needs review and is not confirmed), tags, XML only or skipped, and why; with `--json` as the document's `export` field, an `ExportOutcome` as the app receives it); `labels`, `eval` and `bench` serve the evaluation; `cache path|clear` manages the analysis cache; `--version` prints the version with the git revision and build date.

## File changes

File changes go through the write transaction (`sc_engine::apply_file`, `sc_io::txn`):

- `apply <files> --gain-db <dB> [--trim-samples N] [--bits 16|24] [--tag NAME=VALUE]... [--out <dir>] [--backup-root <dir>] [--no-keep-mtime] [--no-sidecar] [--json]` changes WAV, AIFF and FLAC files in place after backing them up (or writes copies into `--out`), verified. `--trim-samples N` asks for a cut of N frames: it is made up to 1 ms earlier (44 frames at 44.1 kHz, 48 at 48 kHz), at the frame whose loudest channel is quietest (the latest on a tie, never later than N), and the first 2 ms after it fade in (raised cosine from silence); the line says `500 frames trimmed (520 requested)`, and cue points, markers, loops, `bext` TimeReference and the FLAC CUESHEET move by the cut made. Tags use neutral names: `BPM` (ID3 `TBPM` + `TXXX:BPM`, Vorbis `BPM`), `INITIALKEY` (`TKEY` / `INITIALKEY`), any other `NAME` (`TXXX:NAME` / `NAME`); ID3v2.3/2.4 frame ids (and the iTunes ones, e.g. `TCMP`) and labels with `:` stop the run before any file; other four-letter names such as `MOOD` are ordinary names. A file listed twice, and with `--out` a second file with the same output name, is refused before anything is written.
- `process <files> --batch-mode prepare|library [--grid-only] [--out <dir>] [--depth source|16|24] [--lead-ms N] [--no-tbpm] [--mode dj|streaming] [--target LUFS] [--ceiling dBTP] [--backup-root <dir>] [--jobs N] [--json]` exports files exactly as the app's Export button does (the engine's process job): crash recovery runs first, then each file is analysed (the cache is used, the grid edits saved in the app are applied), planned as `plan --batch-mode` plans it, written in place after a backup (or as a copy into `--out`) with its gain, Prepare cut and tags, verified, and analysed again, which refreshes its cache entry; a grid edit saved for it is carried over to the written file, bar 1 moved back by the cut. The output must have the planned frame count (Library: the source's; Prepare: the source's minus the cut) or nothing is replaced. Library keeps the modification time, Prepare does not. The tempo tag is written in Prepare only, unless `--no-tbpm`. One line per file in the order given, worded as `plan` words it: `<path>: prepare: Gain -2.0 dB, Cut 0.21 s; tags BPM, ...; written and verified; backup <path>` (or `; written to <path>`), `<path>: library: XML only: <why>`, or `<path>: prepare: skipped` with `  why:` and `  what to do:` lines; then `N files: W written, X XML only, S skipped, F failed`. The rekordbox XML itself is not written yet, so a file left to it is only listed. The sidecar (schema 2) records the export: settings, the plans, the grid as exported in the output's samples and the source's measurements.
- `undo <files>` puts back the version before each file's newest change.
- `journal [--incomplete] [--forget <id>] [--json]` lists the changes recorded in the backup folder, newest first, or gives up on a pending one (ids look like `6ac5b4f4-75821-0`).
- `recover [--json]` finishes or rolls back changes a crash interrupted; `process`, `apply` and `undo` run it first.

The backup folder is `~/Music/SoundCheck Backups` unless `--backup-root` or `SC_BACKUP_ROOT` names another. Text output: one line per file, starting with the path as given; a refused file prints three lines on stderr (`<path>: not changed|not written|not undone`, `  why: ...`, `  what to do: ...`).

Exit codes: 0 when every file succeeded; 2 when any file failed or was refused (or the arguments are invalid); `recover` exits 3 when changes stay pending.

## JSON, schema 1

Every document has `"schema": 1`. Keys are camelCase; the journal's kinds, states and outcomes use these values:

- kind: `inPlace`, `toFolder`, `undo`
- state: `planned`, `tempWritten`, `verified`, `backedUp`, `renamed`, `metadataDone`, `done`, `failed`, `recovered`, `forgotten`
- outcome: `completed`, `rolledBack`

`apply --json`, one document per file:
- success: `file` (as given), `ok: true`, `txn`, `kind`, `output`, `backup` (in place) or null, `sidecar` or null, `request` {`gainDb`, `trimFrames`, `bits`, `tags`: [{`name`, `value`}]}, `render` {`framesIn`, `framesOut`, `trimFrames` (the cut made), `trimRequestedFrames`, `sampleRateHz`, `channels`, `bitsOut`, `exact`, `dithered`, `samplesSaturated`, `pcmBlake3`, `blocks` {`carried`, `patched`, `edited`, `replaced`, `dropped`}, `tagsAdded`, `tagsReplaced`, `tagsNotAdded`, `staleLoudnessTags`}, `originalBlake3`, `outputBlake3`, `outputBytes`, `notes`, `timings`: [{`step` (a state), `ms`}], `totalMs`.
- failure (also `undo`): `file`, `ok: false`, `error` {`kind` (the IPC error class, e.g. `inPlaceRefused`, `listedTwice`, `sameOutputName`), `message`, `fileId`}, `why`, `whatToDo`.

`process --json`, one document per file not refused: `file`, `ok: true`, `export` (the `ExportOutcome`: `{type: "write", plan}`, `{type: "xmlOnly", reason}` or `{type: "skip", reason}`), and for a file written `written` {`txn`, `output`, `backup`, `sidecar`, `framesIn`, `framesOut`, `trimFrames`, `originalBlake3`, `outputBlake3`, `editCarried`, `gridConfirmed`, `outputAnalysed`, `notes`}; a refusal is the failure document above.

`undo --json`, one document per file: `file`, `ok: true`, `txn`, `undone` (the change undone), `path`, `backup`, `restoredBlake3`, `sidecar` (`removed`, `restored` or `absent`), `earlierChanges`, `notes` (metadata of the original that could not be restored), `totalMs`.

`journal --json`: `backupRoot`, `entries` (newest first): [{`txn`, `startedAt`, `kind`, `state`, `reached`, `outcome`, `undone`, `path`, `source`, `backup`, `error`, `notes`}]. With `--forget`: `ok`, `txn`, `kind`, `path`, `reached`, `backup`, `kept` (files left in place), or `ok: false` with `error`.

`recover --json`: `backupRoot`, `recovered`: [{`txn`, `kind`, `path`, `reached`, `outcome`, `notes`}], `pending`: [{`txn`, `path`, `reason`}].

## Tests

`tests/apply.rs`, `tests/process.rs`, `tests/inputs.rs` and `tests/journal.rs` run these commands in temp folders with `SC_BACKUP_ROOT` there (in place, undo byte for byte across two changes, `--out`, an export in place, to a folder and as JSON, neutral tags into WAV, AIFF and FLAC in one run, JSON, refusals, a file listed twice, output name collisions, listing, recovery of a staged interrupted change, forget, path-like ids).
