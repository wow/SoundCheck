//! The export commands on a click track with its grid (needs the model files; skipped without): a
//! Prepare cut with a confirmed edit, its grid check and rekordbox XML, the revert that brings the
//! original's measurements and edit back, and the file read again at export time.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use sc_core::analysis::{AnalysisSettings, GridEdit, Model};
use sc_core::export::{Cut, ExportSkip, GridCheck, LEAD_MS_RANGE};
use sc_core::ipc::{
    AnalyzeRequest, ExportAction, IpcErrorKind, JobEvent, RowUpdate, WriteXmlRequest,
};
use sc_core::{AudioSpec, Bpm, testsig};
use sc_io::cache::Cache;

use super::support::{
    Lib, date_back, events_until_finished, export, mtime, prepare, request, sha256, steps,
};
use crate::shell::Shell;

fn have_models() -> bool {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../models");
    let ok =
        dir.join("mel_spectrogram.onnx").is_file() && dir.join("beat_this_small.onnx").is_file();
    if !ok {
        eprintln!("skipped: no model files; run scripts/fetch-models.sh");
    }
    ok
}

fn with_grid() -> AnalysisSettings {
    AnalysisSettings {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        grid: true,
        model: Model::Small,
    }
}

/// 0.4 s of silence, then 72 clicks at 124 BPM, 16-bit stereo 44.1 kHz.
fn click_wav(path: &Path) {
    let clicks = testsig::click_track(AudioSpec::CD, Bpm(124.0), 72, 15.0);
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..2 * testsig::frames_for(AudioSpec::CD, 0.4) {
        w.write_sample(0_i16).unwrap();
    }
    for s in &clicks.data {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((f64::from(*s) * 0.5 * 32_767.0).round() as i16)
            .unwrap();
    }
    w.finalize().unwrap();
}

/// Adds the file and analyses it with the grid.
fn analysed_click(shell: &Shell, path: &Path) -> u32 {
    let rows = shell.expand(vec![path.display().to_string()]);
    let id = rows[0].file_id;
    let (tx, rx) = mpsc::channel();
    shell
        .start(
            AnalyzeRequest {
                file_ids: vec![id],
                analysis: with_grid(),
            },
            move |e| {
                let _ = tx.send(e);
            },
        )
        .unwrap();
    events_until_finished(&rx);
    id
}

/// Opens the row in the grid view and confirms its grid, as a user does by ear.
fn confirm(shell: &Shell, id: u32) -> RowUpdate {
    shell.open_track(id, |_| {}).unwrap();
    shell.commit(id, &GridEdit::default(), true).unwrap()
}

#[test]
fn revert_restores_hash_and_the_confirmed_edit() {
    if !have_models() {
        return;
    }
    let lib = Lib::new();
    let wav = lib.music().join("click.wav");
    click_wav(&wav);
    let dated = date_back(&wav);
    let shell = lib.shell(1);
    let id = analysed_click(&shell, &wav);
    let confirmed = confirm(&shell, id);
    assert!(confirmed.row.confirmed);
    let original = confirmed.row.clone();
    let bar1 = original.grid.as_ref().expect("a grid").bar1;
    let before = sha256(&wav);
    let (nfc, key) = Cache::key_for(&wav, &with_grid()).unwrap();
    let cached_before = lib.cache().get(&nfc, &key).expect("cached");

    // The XML command lists the row before any export, leaving the file as it is.
    let xml = lib.base().join("analysed.xml");
    let written = shell
        .write_xml(WriteXmlRequest {
            file_ids: vec![id],
            out: Some(xml.display().to_string()),
        })
        .unwrap();
    assert_eq!((written.artefacts.listed, written.not_listed.len()), (1, 0));
    let text = std::fs::read_to_string(&xml).unwrap();
    assert!(
        text.contains("click.wav") && text.contains("TEMPO"),
        "{text}"
    );
    assert_eq!(sha256(&wav), before);

    let (_, events) = export(&shell, request(&[id], prepare(), None));

    // The track open in the grid view is about the file as it was: it closes.
    assert_eq!(
        shell.peaks(id, 1024, 0, 10).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );
    assert_eq!(
        steps(&events, id),
        ["started", "processing", "written", "exportDone"],
    );
    for event in &events {
        let text = serde_json::to_string(event).unwrap();
        assert!(!text.contains("evidence"), "the evidence stays off IPC");
        assert!(text.len() <= 4096, "{} bytes", text.len());
    }
    let (row, update) = events
        .iter()
        .find_map(|e| match e {
            JobEvent::ExportDone { row, update, .. } => Some((row, update)),
            _ => None,
        })
        .expect("done");
    let ExportAction::Write { cut, .. } = row.action else {
        panic!("{:?}", row.action);
    };
    let Cut::Cut { seconds, .. } = cut else {
        panic!("{cut:?}");
    };
    assert!(seconds.0 > 0.3, "{cut:?}");
    assert!(
        row.positions_shift && row.edit_carried && row.revertible,
        "{row:?}"
    );
    assert!(
        matches!(row.grid_check, Some(GridCheck::Pass { .. })),
        "{:?}",
        row.grid_check
    );
    let v = row.verification.as_ref().expect("verified");
    let Cut::Cut { frames, .. } = cut else {
        panic!("{cut:?}");
    };
    assert_eq!(v.frames_in - v.frames_out, frames, "the cut, exactly");
    // The row follows the written file: bar 1 a lead after its start, and the edit carried.
    let update = update.as_ref().expect("the row follows");
    assert!(update.row.confirmed, "the confirmed edit is carried over");
    let moved = update.row.grid.as_ref().expect("a grid").bar1.0;
    assert!((0.0..0.012).contains(&moved), "bar 1 at {moved} s");
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::Artefacts { artefacts, .. } if artefacts.listed == 1
    )));
    assert_ne!(sha256(&wav), before);

    let reverted = shell.revert(id).unwrap();

    assert_eq!(sha256(&wav), before, "the original's bytes");
    assert_eq!(mtime(&wav), dated, "the original's modification time");
    let update = reverted.update.expect("analysed again");
    assert_eq!(update.row.short_term_p95, original.short_term_p95);
    assert_eq!(update.row.integrated, original.integrated);
    assert_eq!(update.row.true_peak, original.true_peak);
    assert!(update.row.confirmed, "the original's edit is back");
    assert_eq!(update.row.grid.as_ref().expect("a grid").bar1, bar1);
    let (nfc, key) = Cache::key_for(&wav, &with_grid()).unwrap();
    let cached = lib.cache().get(&nfc, &key).expect("the original is cached");
    assert_eq!(cached.loudness, cached_before.loudness);
    let snapshot = shell.restore();
    assert_eq!(snapshot.rows[0].export, None);
    assert_eq!(
        snapshot.rows[0].row.as_ref().unwrap().duration,
        original.duration
    );
}

/// Appends an ID3 chunk holding a Serato cue object to a RIFF file made of `fmt ` and `data`.
fn add_serato_tag(path: &Path) {
    let mut frame_body = vec![0_u8];
    frame_body.extend_from_slice(b"application/octet-stream\0\0Serato Markers_\0\x02\x05");
    let mut frames = Vec::new();
    frames.extend_from_slice(b"GEOB");
    frames.extend_from_slice(&u32::try_from(frame_body.len()).unwrap().to_be_bytes());
    frames.extend_from_slice(&[0, 0]);
    frames.extend_from_slice(&frame_body);
    let size = u32::try_from(frames.len()).unwrap();
    let mut tag = b"ID3\x03\x00\x00".to_vec();
    tag.extend_from_slice(&[
        ((size >> 21) & 0x7f) as u8,
        ((size >> 14) & 0x7f) as u8,
        ((size >> 7) & 0x7f) as u8,
        (size & 0x7f) as u8,
    ]);
    tag.extend_from_slice(&frames);
    if tag.len() % 2 == 1 {
        tag.push(0);
    }
    let mut chunk = b"id3 ".to_vec();
    chunk.extend_from_slice(&u32::try_from(tag.len()).unwrap().to_le_bytes());
    chunk.extend_from_slice(&tag);
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(&chunk).unwrap();
    drop(f);
    // The RIFF size counts what was appended.
    let mut bytes = std::fs::read(path).unwrap();
    let riff = u32::try_from(bytes.len() - 8).unwrap();
    bytes[4..8].copy_from_slice(&riff.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn an_export_reads_the_file_again_so_serato_data_added_since_the_row_was_probed_counts() {
    if !have_models() {
        return;
    }
    let lib = Lib::new();
    let wav: PathBuf = lib.music().join("click.wav");
    click_wav(&wav);
    let shell = lib.shell(1);
    let id = analysed_click(&shell, &wav);
    confirm(&shell, id);
    shell.close_track();
    assert!(
        !shell.restore().rows[0].entry.info.serato,
        "none when added"
    );

    add_serato_tag(&wav);
    assert!(sc_io::probe(&wav).serato, "the file now holds Serato data");
    let before = sha256(&wav);

    let (_, events) = export(&shell, request(&[id], prepare(), None));

    assert_eq!(steps(&events, id), ["started", "exportSkipped"]);
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::ExportSkipped { row, .. } if matches!(
            row.action,
            ExportAction::Skip { reason: ExportSkip::SeratoInPlaceCut { .. } }
        )
    )));
    assert_eq!(
        sha256(&wav),
        before,
        "a cut in place would move its cue points"
    );
}

#[test]
fn the_lowest_lead_keeps_bar_1_out_of_the_fade_in_the_audio() {
    if !have_models() {
        return;
    }
    let lib = Lib::new();
    let wav = lib.music().join("click.wav");
    click_wav(&wav);
    let shell = lib.shell(1);
    let id = analysed_click(&shell, &wav);
    confirm(&shell, id);
    shell.close_track();
    let floor = sc_core::export::ExportSettings {
        lead_ms: LEAD_MS_RANGE.0,
        ..prepare()
    };
    let (_, events) = export(&shell, request(&[id], floor, None));
    assert!(steps(&events, id).contains(&"exportDone"));
    // The first click, as loud as it was: not faded. Its first sample after the 88-frame fade.
    let mut reader = hound::WavReader::open(&wav).unwrap();
    let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    let fade_frames = sc_io::render::head_fade_frames(44_100);
    let peak_in_fade = samples[..2 * fade_frames]
        .iter()
        .map(|s| s.unsigned_abs())
        .max()
        .unwrap();
    let peak_after = samples[2 * fade_frames..2 * 2000]
        .iter()
        .map(|s| s.unsigned_abs())
        .max()
        .unwrap();
    assert!(peak_after > 1000, "{peak_after}");
    assert!(
        peak_in_fade < peak_after / 2,
        "{peak_in_fade} in the fade, {peak_after} after"
    );
}
