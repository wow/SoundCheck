//! Helpers of the export commands' tests: temp folders for the music, backups, cache, edits and
//! exports, a shell over them, synthetic tones, and runs of a job to its end.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use sc_core::analysis::{AnalysisSettings, Model};
use sc_core::export::{BatchMode, ExportSettings, Place};
use sc_core::ipc::{AnalyzeRequest, ExportRequest, JobEvent, JobId};
use sc_core::{AudioSpec, Bpm, testsig};
use sc_io::cache::Cache;
use sc_io::edits::EditStore;
use sc_io::txn;
use sha2::{Digest, Sha256};

use crate::shell::{Roots, Shell};

/// One test's folders and a shell over them.
pub(super) struct Lib {
    dir: tempfile::TempDir,
}

impl Lib {
    pub(super) fn new() -> Self {
        let lib = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(lib.music()).unwrap();
        lib
    }
    pub(super) fn base(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }
    pub(super) fn music(&self) -> PathBuf {
        self.base().join("music")
    }
    pub(super) fn backups(&self) -> PathBuf {
        self.base().join("backups")
    }
    pub(super) fn exports(&self) -> PathBuf {
        self.base().join("exports")
    }
    pub(super) fn cache(&self) -> Cache {
        Cache::open(self.base().join("cache"))
    }
    /// A shell whose start-up recovery has not ended.
    pub(super) fn shell_recovering(&self, workers: usize) -> Shell {
        Shell::with_roots(
            Some(self.cache()),
            Some(EditStore::open(self.base().join("edits"))),
            workers,
            Roots {
                backup: Some(self.backups()),
                exports: Some(self.exports()),
            },
        )
    }
    /// A shell whose start-up recovery has ended.
    pub(super) fn shell(&self, workers: usize) -> Shell {
        let shell = self.shell_recovering(workers);
        shell
            .recover_in_background()
            .expect("a recovery thread")
            .join()
            .expect("recovery does not panic");
        assert!(!shell.inner.recovery.is_running());
        shell
    }
    pub(super) fn tone(&self, name: &str, amp: f32) -> PathBuf {
        self.tone_at(name, amp, 44_100)
    }
    pub(super) fn tone_at(&self, name: &str, amp: f32, rate: u32) -> PathBuf {
        let path = self.music().join(name);
        tone_wav(&path, amp, rate);
        path
    }
}

/// 5 s of a 1 kHz tone, 16-bit stereo.
pub(super) fn tone_wav(path: &Path, amp: f32, rate: u32) {
    let audio = AudioSpec {
        sample_rate: rate,
        channels: 2,
    };
    let buf = testsig::sine(audio, 1000.0, amp, 5.0);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for s in &buf.data {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((f64::from(*s) * f64::from(i16::MAX)).round() as i16)
            .unwrap();
    }
    w.finalize().unwrap();
}

/// The SHA-256 of the file, the hash the release checklist's revert is stated in.
pub(super) fn sha256(path: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(path).unwrap()).to_vec()
}

pub(super) fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

/// Dates the file an hour back, so a write and a restore both show.
pub(super) fn date_back(path: &Path) -> SystemTime {
    let then = SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(then)
        .unwrap();
    mtime(path)
}

pub(super) fn loudness_only() -> AnalysisSettings {
    AnalysisSettings {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        grid: false,
        model: Model::Small,
    }
}

pub(super) fn events_until_finished(rx: &mpsc::Receiver<JobEvent>) -> Vec<JobEvent> {
    let mut events = Vec::new();
    loop {
        let e = rx
            .recv_timeout(Duration::from_secs(120))
            .expect("the job ends");
        let done = matches!(e, JobEvent::Finished { .. });
        events.push(e);
        if done {
            return events;
        }
    }
}

/// Adds the files and analyses them, loudness only.
pub(super) fn analysed(shell: &Shell, files: &[&Path]) -> Vec<u32> {
    let rows = shell.expand(files.iter().map(|f| f.display().to_string()).collect());
    let ids: Vec<u32> = rows.iter().map(|r| r.file_id).collect();
    let (tx, rx) = mpsc::channel();
    shell
        .start(
            AnalyzeRequest {
                file_ids: ids.clone(),
                analysis: loudness_only(),
            },
            move |e| {
                let _ = tx.send(e);
            },
        )
        .unwrap();
    events_until_finished(&rx);
    ids
}

pub(super) fn request(
    file_ids: &[u32],
    settings: ExportSettings,
    out_dir: Option<&Path>,
) -> ExportRequest {
    ExportRequest {
        file_ids: file_ids.to_vec(),
        settings,
        out_dir: out_dir.map(|d| d.display().to_string()),
        confirmed: None,
    }
}

/// Runs an export to its end.
pub(super) fn export(shell: &Shell, req: ExportRequest) -> (JobId, Vec<JobEvent>) {
    let (tx, rx) = mpsc::channel();
    let job = shell
        .export(req, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
    (job, events_until_finished(&rx))
}

/// A job whose events stop being forwarded at one of them, so it cannot send `finished`, and so
/// lets go of nothing, until released.
pub(super) struct Held {
    rx: mpsc::Receiver<JobEvent>,
    release: mpsc::Sender<()>,
    /// The events up to the one it stopped at.
    pub(super) before: Vec<JobEvent>,
}

impl Held {
    /// Lets the job go on, and returns all its events to the end.
    pub(super) fn finish(self) -> Vec<JobEvent> {
        self.release.send(()).unwrap();
        let mut events = self.before;
        events.extend(events_until_finished(&self.rx));
        events
    }
}

/// How a test starts a job: its id, or why it was refused.
type StartJob<'a> =
    &'a dyn Fn(Box<dyn FnMut(JobEvent) + Send>) -> Result<JobId, sc_core::ipc::IpcError>;

/// Starts a job with `start`, held at its first event that `stop` accepts.
fn hold_at(start: StartJob<'_>, stop: fn(&JobEvent) -> bool) -> Held {
    let (tx, rx) = mpsc::channel();
    let (release, gate) = mpsc::channel::<()>();
    let mut stopped = false;
    start(Box::new(move |e| {
        let stop_here = !stopped && stop(&e);
        let _ = tx.send(e);
        if stop_here {
            stopped = true;
            let _ = gate.recv();
        }
    }))
    .unwrap();
    let mut before = Vec::new();
    loop {
        let e = rx
            .recv_timeout(Duration::from_secs(120))
            .expect("the job reaches the event");
        let here = stop(&e);
        before.push(e);
        if here {
            return Held {
                rx,
                release,
                before,
            };
        }
    }
}

/// An export held at its first event that `stop` accepts.
pub(super) fn held_at(shell: &Shell, req: ExportRequest, stop: fn(&JobEvent) -> bool) -> Held {
    hold_at(&|send| shell.export(req.clone(), send), stop)
}

/// An analysis held at its first event that `stop` accepts.
pub(super) fn analysis_held_at(
    shell: &Shell,
    file_ids: &[u32],
    stop: fn(&JobEvent) -> bool,
) -> Held {
    let req = AnalyzeRequest {
        file_ids: file_ids.to_vec(),
        analysis: loudness_only(),
    };
    hold_at(&|send| shell.start(req.clone(), send), stop)
}

/// Prepare, in place: the tests choose a folder with [`in_folder`].
pub(super) fn prepare() -> ExportSettings {
    ExportSettings {
        place: Place::InPlace,
        ..ExportSettings::new(BatchMode::Prepare)
    }
}

pub(super) fn in_folder(settings: ExportSettings) -> ExportSettings {
    ExportSettings {
        place: Place::Folder,
        ..settings
    }
}

/// The names of a file's export events, in order.
pub(super) fn steps(events: &[JobEvent], id: u32) -> Vec<&'static str> {
    events
        .iter()
        .filter_map(|e| match e {
            JobEvent::Started { file_id, .. } if *file_id == id => Some("started"),
            JobEvent::Processing { file_id, .. } if *file_id == id => Some("processing"),
            JobEvent::Written { file_id, .. } if *file_id == id => Some("written"),
            JobEvent::ExportSkipped { file_id, .. } if *file_id == id => Some("exportSkipped"),
            JobEvent::ExportDone { file_id, .. } if *file_id == id => Some("exportDone"),
            JobEvent::Failed { file_id, .. } if *file_id == id => Some("failed"),
            JobEvent::Cancelled { file_id, .. } if *file_id == id => Some("cancelled"),
            _ => None,
        })
        .collect()
}

pub(super) fn no_temp_left(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        assert!(
            !name.to_string_lossy().contains(txn::TEMP_MARKER),
            "{name:?} left in {}",
            dir.display()
        );
        if entry.file_type().unwrap().is_dir() {
            no_temp_left(&entry.path());
        }
    }
}
