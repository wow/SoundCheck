# sc-cli

The headless SoundCheck binary. `sc-cli analyze <file> --json` prints exactly the fields the desktop table shows; `plan` says what processing would do to each file; `labels`, `eval` and `bench` serve the evaluation; `cache path|clear` manages the analysis cache; `--version` prints the version with the git revision and build date.

## File changes

File changes go through the write transaction (`sc_engine::apply_file`, `sc_io::txn`):

- `apply <files> --gain-db <dB> [--trim-samples N] [--bits 16|24] [--tag NAME=VALUE]... [--out <dir>] [--backup-root <dir>] [--no-keep-mtime] [--no-sidecar] [--json]` changes WAV, AIFF and FLAC files in place after backing them up (or writes copies into `--out`), verified. Tags use neutral names: `BPM` (ID3 `TBPM` + `TXXX:BPM`, Vorbis `BPM`), `INITIALKEY` (`TKEY` / `INITIALKEY`), any other `NAME` (`TXXX:NAME` / `NAME`); ID3 frame ids and labels with `:` stop the run before any file. A file listed twice, and with `--out` a second file with the same output name, is refused before anything is written.
- `undo <files>` puts back the version before each file's newest change.
- `journal [--incomplete] [--forget <id>] [--json]` lists the changes recorded in the backup folder, newest first, or gives up on a pending one (ids look like `6ac5b4f4-75821-0`).
- `recover [--json]` finishes or rolls back changes a crash interrupted; `apply` and `undo` run it first.

The backup folder is `~/Music/SoundCheck Backups` unless `--backup-root` or `SC_BACKUP_ROOT` names another. Text output: one line per file, starting with the path as given; a refused file prints three lines on stderr (`<path>: not changed|not written|not undone`, `  why: ...`, `  what to do: ...`).

Exit codes: 0 when every file succeeded; 2 when any file failed or was refused (or the arguments are invalid); `recover` exits 3 when changes stay pending.

## JSON, schema 1

Every document has `"schema": 1`. Keys are camelCase; the journal's kinds, states and outcomes use these values:

- kind: `inPlace`, `toFolder`, `undo`
- state: `planned`, `tempWritten`, `verified`, `backedUp`, `renamed`, `metadataDone`, `done`, `failed`, `recovered`, `forgotten`
- outcome: `completed`, `rolledBack`

`apply --json`, one document per file:
- success: `file` (as given), `ok: true`, `txn`, `kind`, `output`, `backup` (in place) or null, `sidecar` or null, `request` {`gainDb`, `trimFrames`, `bits`, `tags`: [{`name`, `value`}]}, `render` {`framesIn`, `framesOut`, `sampleRateHz`, `channels`, `bitsOut`, `exact`, `dithered`, `samplesSaturated`, `pcmBlake3`, `blocks` {`carried`, `patched`, `edited`, `replaced`, `dropped`}, `tagsAdded`, `tagsReplaced`, `tagsNotAdded`, `staleLoudnessTags`}, `originalBlake3`, `outputBlake3`, `outputBytes`, `notes`, `timings`: [{`step` (a state), `ms`}], `totalMs`.
- failure (also `undo`): `file`, `ok: false`, `error` {`kind` (the IPC error class, e.g. `inPlaceRefused`, `listedTwice`, `sameOutputName`), `message`, `fileId`}, `why`, `whatToDo`.

`undo --json`, one document per file: `file`, `ok: true`, `txn`, `undone` (the change undone), `path`, `backup`, `restoredBlake3`, `sidecar` (`removed`, `restored` or `absent`), `earlierChanges`, `totalMs`.

`journal --json`: `backupRoot`, `entries` (newest first): [{`txn`, `startedAt`, `kind`, `state`, `reached`, `outcome`, `undone`, `path`, `source`, `backup`, `error`, `notes`}]. With `--forget`: `ok`, `txn`, `kind`, `path`, `reached`, `backup`, `kept` (files left in place), or `ok: false` with `error`.

`recover --json`: `backupRoot`, `recovered`: [{`txn`, `kind`, `path`, `reached`, `outcome`, `notes`}], `pending`: [{`txn`, `path`, `reason`}].

## Tests

`tests/apply.rs`, `tests/inputs.rs` and `tests/journal.rs` run these commands in temp folders with `SC_BACKUP_ROOT` there (in place, undo byte for byte across two changes, `--out`, neutral tags into WAV, AIFF and FLAC in one run, JSON, refusals, a file listed twice, output name collisions, listing, recovery of a staged interrupted change, forget, path-like ids).
