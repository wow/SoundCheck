//! Throughput of the batch artefacts: a rekordbox XML and a grid report of 1,000 tracks with
//! Turkish names (percent-encoded locations, escaped names, one `TEMPO` each), in memory.
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use std::path::PathBuf;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use sc_core::analysis::Meter;
use sc_core::{Bpm, SampleIndex, Seconds};
use sc_io::rekordbox::{Tempo, XmlTrack, xml_bytes};
use sc_io::report::{ReportRow, csv_bytes};

const TRACKS: u64 = 1_000;

fn tracks() -> Vec<XmlTrack> {
    (0..TRACKS)
        .map(|i| XmlTrack {
            path: PathBuf::from(format!(
                "/Users/dj/Müzik/Şarkılar & Co/{i:04} Deniz'in Parçası.aiff"
            )),
            title: Some(format!("Gece Yarısı {i}")),
            artist: Some("Ayşe & İlhan".into()),
            duration: Seconds(360.0),
            tempo: Tempo::of_bar1(
                SampleIndex(221 + i),
                Bpm(128.0),
                &Meter::four_four(),
                44_100,
            )
            .expect("4/4"),
        })
        .collect()
}

fn rows() -> Vec<ReportRow> {
    (0..TRACKS)
        .map(|i| ReportRow {
            file: format!("/Users/dj/Müzik/Şarkılar, \"Co\"/{i:04}.aiff"),
            mode: "prepare".into(),
            action: "written".into(),
            gain_db: Some(-2.3),
            cut: Some(Seconds(0.21)),
            bpm: Some(Bpm(128.0)),
            bar1: Some(Seconds(0.005)),
            grid: "tempo: beat 1 at 0.005 s".into(),
            grid_check: None,
            notes: vec!["tags BPM, SOUNDCHECK".into()],
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("artefacts");
    g.throughput(Throughput::Elements(TRACKS));
    let t = tracks();
    g.bench_function("rekordbox xml, 1000 tracks", |b| {
        b.iter(|| xml_bytes(std::hint::black_box(&t), "0.0.0", "SoundCheck bench"));
    });
    let r = rows();
    g.bench_function("grid report, 1000 rows", |b| {
        b.iter(|| csv_bytes(std::hint::black_box(&r)));
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
