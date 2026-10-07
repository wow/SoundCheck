# sc-cli

The headless SoundCheck binary. `sc-cli analyze <file> --json` prints exactly the fields the desktop table shows; `plan` says what processing would do to each file; `labels`, `eval` and `bench` serve the evaluation; `cache path|clear` manages the analysis cache; `--version` prints the version with the git revision and build date.

File changes go through the write transaction (`sc_engine::apply_file`, `sc_io::txn`):

- `apply <files> --gain-db <dB> [--trim-samples N] [--bits 16|24] [--tag LABEL=VALUE]... [--out <dir>] [--backup-root <dir>] [--no-keep-mtime] [--no-sidecar] [--json]` changes WAV, AIFF and FLAC files in place after backing them up (or writes copies into `--out`), verified; one line per file, or three lines on stderr (what / why / what to do) when a file is refused.
- `undo <files>` puts back the original of each file's newest change.
- `journal [--incomplete] [--forget <id>] [--json]` lists the changes recorded in the backup folder, newest first, or gives up on a pending one.
- `recover [--json]` finishes or rolls back changes a crash interrupted; `apply` and `undo` run it first.

The backup folder is `~/Music/SoundCheck Backups` unless `--backup-root` or `SC_BACKUP_ROOT` names another. JSON documents of these commands carry `"schema": 1`. A file that fails makes the exit code 2.

Tests: `tests/apply.rs` and `tests/journal.rs` run these commands in temp folders with `SC_BACKUP_ROOT` there (in place, undo byte for byte, `--out`, tags, JSON, refusals, listing, recovery of a staged interrupted change, forget).
