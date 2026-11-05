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
use sc_core::{Bpm, testsig};
use sc_io::cache::Cache;

use super::super::Shell;
use super::support::{
    Lib, date_back, events_until_finished, export, held_at, in_folder, mtime, prepare, request,
    sha256, steps,
};

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

/// 0.3 s of a quiet noise floor, then 64 beats at 120 BPM of kick and click with an accented bar 1
/// that the detector finds wherever the track starts. 16-bit stereo 44.1 kHz.
fn click_wav(path: &Path) {
    let mono = testsig::kick_click_track(120.0, 64, 0.3, testsig::KICK_CLICK_ACCENTS);
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: testsig::KICK_CLICK_RATE_HZ,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for s in mono {
        #[allow(clippy::cast_possible_truncation)]
        let q = (s * f64::from(i16::MAX)).round() as i16;
        w.write_sample(q).unwrap();
        w.write_sample(q).unwrap();
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

/// A Prepare cut with a confirmed edit, as its events show it.
fn the_cut_row_shows_the_export(events: &[JobEvent], wav: &Path) {
    for event in events {
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
    assert!(seconds.0 > 0.2, "{cut:?}");
    assert!(
        row.positions_shift && row.edit_carried && row.revertible,
        "{row:?}"
    );
    // The written file's own analysis agrees with the exported grid, and the row says what the
    // sidecar recorded.
    let doc = sc_io::txn::sidecar::read(&sc_io::txn::sidecar_path(wav)).unwrap();
    let recorded = doc.export.expect("the export is recorded").grid_check;
    let Some(GridCheck::Pass { offset_ms, .. }) = recorded else {
        panic!("{recorded:?}");
    };
    assert!(offset_ms.abs() <= 2.0, "{offset_ms} ms");
    assert_eq!(row.grid_check, recorded);
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

    // While the export runs, a grid view of the old version cannot open (see `holds.rs`); one
    // already open is about the file as it was: it closes.
    let (_, events) = export(&shell, request(&[id], prepare(), None));

    assert_eq!(
        shell.peaks(id, 1024, 0, 10).unwrap_err().kind,
        IpcErrorKind::InvalidArgument
    );
    assert_eq!(
        steps(&events, id),
        ["started", "processing", "written", "exportDone"],
    );
    the_cut_row_shows_the_export(&events, &wav);
    assert_ne!(sha256(&wav), before);

    // A grid view opened on the written file is about what the revert takes away.
    shell.open_track(id, |_| {}).unwrap();
    shell.peaks(id, 1024, 0, 10).expect("it is open");
    let reverted = shell.revert(id).unwrap();
    assert_eq!(
        shell.peaks(id, 1024, 0, 10).unwrap_err().kind,
        IpcErrorKind::InvalidArgument,
        "the revert closed the view"
    );

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
fn the_confirm_step_plans_the_cut_the_job_makes() {
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

    let preview = shell.plan_export(&request(&[id], prepare(), None)).unwrap();
    let row = preview.rows[0].row.as_ref().expect("planned");
    assert!(row.positions_shift, "{row:?}");
    assert!(matches!(
        row.action,
        ExportAction::Write {
            cut: Cut::Cut { .. },
            ..
        }
    ));

    let (_, events) = export(&shell, request(&[id], prepare(), None));
    let planned = events.iter().find_map(|e| match e {
        JobEvent::Processing { row, .. } => Some(row.action.clone()),
        _ => None,
    });
    assert_eq!(
        planned.as_ref(),
        Some(&row.action),
        "the cut, to the sample"
    );
}

#[test]
fn the_xml_button_lists_an_exported_copy_where_it_is() {
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
    let out = lib.base().join("copies");
    let (_, events) = export(&shell, request(&[id], in_folder(prepare()), Some(&out)));
    assert!(steps(&events, id).contains(&"exportDone"), "{events:?}");

    let xml = lib.base().join("rebuilt.xml");
    let written = shell
        .write_xml(WriteXmlRequest {
            file_ids: vec![id],
            out: Some(xml.display().to_string()),
        })
        .unwrap();

    assert_eq!(written.artefacts.listed, 1);
    let text = std::fs::read_to_string(&xml).unwrap();
    assert!(text.contains("copies/click.wav"), "{text}");
    assert!(!text.contains("music/click.wav"), "{text}");
}

#[test]
fn a_saved_grid_marks_the_export_stale_only_when_it_differs() {
    if !have_models() {
        return;
    }
    let lib = Lib::new();
    let wav = lib.music().join("click.wav");
    click_wav(&wav);
    let shell = lib.shell(1);
    let id = analysed_click(&shell, &wav);
    assert!(!confirm(&shell, id).export_stale, "no export yet");
    shell.close_track();
    let out = lib.base().join("copies");
    export(&shell, request(&[id], in_folder(prepare()), Some(&out)));
    assert!(!shell.restore().rows[0].export_stale);

    // Confirmed again as it was: nothing new to write.
    assert!(!confirm(&shell, id).export_stale);
    shell.close_track();
    assert!(!shell.restore().rows[0].export_stale);

    // Another bar 1: the export no longer says what the row says, also after a reload.
    shell.open_track(id, |_| {}).unwrap();
    let edit = GridEdit {
        anchor: Some(sc_core::SampleIndex(20_000)),
        ..GridEdit::default()
    };
    assert!(shell.commit(id, &edit, true).unwrap().export_stale);
    shell.close_track();
    assert!(shell.restore().rows[0].export_stale);
}

#[test]
fn a_grid_view_opens_during_a_copy_to_a_folder_whose_source_does_not_change() {
    if !have_models() {
        return;
    }
    let lib = Lib::new();
    let wav = lib.music().join("click.wav");
    click_wav(&wav);
    let shell = lib.shell(1);
    let id = analysed_click(&shell, &wav);
    let out = lib.base().join("out");
    let held = held_at(
        &shell,
        request(&[id], in_folder(prepare()), Some(&out)),
        |e| matches!(e, JobEvent::Started { .. }),
    );
    shell
        .open_track(id, |_| {})
        .expect("the source stays as it is");
    held.finish();
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
