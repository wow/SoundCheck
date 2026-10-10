//! Helpers of the export tests (`process.rs`, `process_grid.rs`): a temp library with its
//! backups, cache and edits, process settings, event names, synthetic WAVs and file checks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sc_core::analysis::AnalysisSettings;
use sc_core::export::{BatchMode, ExportSettings, Place};
use sc_core::ipc::RecoveryStatus;
use sc_core::plan::DecideSettings;
use sc_core::{AudioSpec, testsig};
use sc_engine::{
    BatchFile, BatchSettings, CancelToken, EngineEvent, ProcessDone, ProcessSettings, RecoveryGate,
    Task, run_batch,
};
use sc_io::cache::Cache;
use sc_io::edits::EditStore;
use sc_io::txn::{self, TEMP_MARKER, sidecar};

/// Folders of one test: the music, backups, cache, edits.
pub struct Lib {
    pub dir: tempfile::TempDir,
}

impl Lib {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }
    pub fn music(&self) -> PathBuf {
        let p = self.dir.path().join("music");
        std::fs::create_dir_all(&p).expect("music folder");
        p
    }
    pub fn backups(&self) -> PathBuf {
        self.dir.path().join("backups")
    }
    pub fn cache(&self) -> Cache {
        Cache::open(self.dir.path().join("cache"))
    }
    pub fn edits(&self) -> EditStore {
        EditStore::open(self.dir.path().join("edits"))
    }
}

pub fn batch(paths: &[PathBuf]) -> Vec<BatchFile> {
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| BatchFile {
            file_id: u32::try_from(i).expect("few files") + 1,
            path: p.clone(),
            duration_hint: None,
        })
        .collect()
}

/// Process settings for `lib` in `mode`, in place unless `out` is given, with the gate open.
pub fn process(lib: &Lib, mode: BatchMode, out: Option<PathBuf>) -> ProcessSettings {
    ProcessSettings {
        decide: DecideSettings::dj(),
        export: ExportSettings {
            place: if out.is_some() {
                Place::Folder
            } else {
                Place::InPlace
            },
            ..ExportSettings::new(mode)
        },
        out_dir: out,
        backup_root: lib.backups(),
        edits: Some(lib.edits()),
        recovery: Arc::new(RecoveryGate::new(finished())),
    }
}

pub fn finished() -> RecoveryStatus {
    RecoveryStatus::Finished {
        recovered: Vec::new(),
        pending: Vec::new(),
    }
}

pub fn settings(lib: &Lib, analysis: AnalysisSettings, p: ProcessSettings) -> BatchSettings {
    BatchSettings {
        analysis,
        workers: 2,
        cache: Some(lib.cache()),
        task: Task::Process(Box::new(p)),
    }
}

pub fn run(files: &[BatchFile], s: &BatchSettings, cancel: &CancelToken) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    run_batch(files, s, cancel, &mut |e| events.push(e)).expect("batch starts");
    events
}

pub fn file_of(event: &EngineEvent) -> Option<u32> {
    match event {
        EngineEvent::Started { file_id }
        | EngineEvent::Progress { file_id, .. }
        | EngineEvent::Analysed { file_id, .. }
        | EngineEvent::Failed { file_id, .. }
        | EngineEvent::Cancelled { file_id }
        | EngineEvent::Processing { file_id, .. }
        | EngineEvent::Written { file_id, .. }
        | EngineEvent::ExportSkipped { file_id, .. }
        | EngineEvent::Done { file_id, .. } => Some(*file_id),
        EngineEvent::Batch(_) => None,
    }
}

/// The names of `file`'s events in order, progress left out.
pub fn names(events: &[EngineEvent], file: u32) -> Vec<&'static str> {
    events
        .iter()
        .filter(|e| file_of(e) == Some(file))
        .filter_map(|e| match e {
            EngineEvent::Started { .. } => Some("started"),
            EngineEvent::Progress { .. } | EngineEvent::Batch(_) => None,
            EngineEvent::Analysed { .. } => Some("analysed"),
            EngineEvent::Failed { .. } => Some("failed"),
            EngineEvent::Cancelled { .. } => Some("cancelled"),
            EngineEvent::Processing { .. } => Some("processing"),
            EngineEvent::Written { .. } => Some("written"),
            EngineEvent::ExportSkipped { .. } => Some("exportSkipped"),
            EngineEvent::Done { .. } => Some("done"),
        })
        .collect()
}

/// A 16-bit stereo WAV of a 1 kHz tone at `rate`.
pub fn tone_at(dir: &Path, name: &str, rate: u32, seconds: f64) -> PathBuf {
    let path = dir.join(name);
    let buf = testsig::sine(AudioSpec::new(rate, 2), 1000.0, 0.1, seconds);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        let q = (f64::from(*s) * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).expect("write");
    }
    w.finalize().expect("finalize");
    path
}

pub fn frames(path: &Path) -> u32 {
    hound::WavReader::open(path).expect("wav").duration()
}

pub fn hash(path: &Path) -> [u8; 32] {
    txn::hash_file(path).expect("hash").1
}

/// Every file under `dir` whose name marks a transaction's temp file.
pub fn temps(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_string_lossy().contains(TEMP_MARKER) {
                found.push(p);
            }
        }
    }
    found
}

pub fn sidecar_of(path: &Path) -> sidecar::SidecarDoc {
    sidecar::read(&txn::sidecar_path(path)).expect("sidecar reads")
}

/// A 120 BPM click track (bar 1 0.3 s in) analysed with the grid, and its cache.
pub fn click(lib: &Lib) -> PathBuf {
    super::click_wav(&lib.music(), 120.0, 64, 0.3)
}

/// The first sample index (frames) whose amplitude passes 1 % of full scale.
pub fn first_sound(path: &Path) -> u64 {
    let mut reader = hound::WavReader::open(path).expect("wav");
    let channels = u64::from(reader.spec().channels);
    let at = reader
        .samples::<i16>()
        .position(|s| s.expect("sample").unsigned_abs() > 327)
        .expect("sound");
    at as u64 / channels
}

/// The `Done` event's payload in `events`.
pub fn done_of(events: &[EngineEvent]) -> &ProcessDone {
    events
        .iter()
        .find_map(|e| match e {
            EngineEvent::Done { done, .. } => Some(&**done),
            _ => None,
        })
        .expect("done")
}
