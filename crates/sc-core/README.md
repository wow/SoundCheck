# sc-core

Types shared by every SoundCheck crate: `AudioSpec`/`AudioBuffer` (interleaved `f32`), unit newtypes (`Lufs`, `Lu`, `DbTp`, `DbFs`, `Bpm`, `SampleIndex`, `Seconds`), the `Error` enum with one variant per failure class the UI distinguishes, and the IPC types whose TypeScript bindings are generated into `src/lib/ipc/generated/`.
Invariants: sample positions are `u64` indices at the file's native rate; seconds are derived. No I/O in this crate. Feature `testsig` adds deterministic synthetic signals for tests.
