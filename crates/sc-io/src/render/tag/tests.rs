//! Unit tests of `crates/sc-io/src/render/tag.rs`: the ID3 chunk of a WAV or AIFF render is
//! edited in place, and a file with no tag, two tags or a tag that cannot be edited renders
//! exactly as without edits and reports why.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sc_core::{Error, RenderRequest, TagEdit};

use super::*;
use crate::id3::test_build::{tag, text};
use crate::iff::test_build::{Form, comm, fmt_pcm, ssnd};
use crate::iff::{Chunk, read_header};
use crate::render::BlockFate::{Carried, Edited, Replaced};
use crate::render::{BlockFate, RenderReport, apply_iff};

fn request(edits: &[(&str, &str)]) -> RenderRequest {
    RenderRequest {
        tag_edits: edits
            .iter()
            .map(|(l, v)| TagEdit {
                label: (*l).into(),
                value: (*v).into(),
            })
            .collect(),
        ..RenderRequest::default()
    }
}

fn ours() -> RenderRequest {
    request(&[("TBPM", "128"), ("TXXX:SOUNDCHECK", "x")])
}

/// Renders `bytes` (a file with extension `ext`) with `req`; the output bytes and report.
fn render(
    bytes: &[u8],
    ext: &str,
    req: &RenderRequest,
) -> sc_core::Result<(Vec<u8>, RenderReport)> {
    let dir = tempfile::tempdir().expect("temp dir");
    let (input, output): (PathBuf, PathBuf) = (
        dir.path().join(format!("in.{ext}")),
        dir.path().join(format!("out.{ext}")),
    );
    std::fs::write(&input, bytes).expect("write input");
    let report = apply_iff(&input, &output, req, &AtomicBool::new(false))?;
    Ok((std::fs::read(&output).expect("output"), report))
}

/// Payload of chunk `i` of a file in memory.
fn payload(bytes: &[u8], i: usize) -> (&[u8], [u8; 4]) {
    let h = read_header(&mut Cursor::new(bytes), Path::new("t")).expect("valid file");
    let c = &h.table.chunks[i];
    let range = usize::try_from(c.payload.start).expect("small")
        ..usize::try_from(c.payload.end).expect("small");
    (&bytes[range], c.id)
}

fn small_tag(padding: usize) -> Vec<u8> {
    tag(
        3,
        0,
        &[],
        &[text(3, *b"TIT2", 0, "A"), text(3, *b"TBPM", 0, "127")],
        padding,
    )
}

#[test]
fn a_wav_id3_chunk_is_edited_in_place_and_keeps_its_id() {
    let id3 = small_tag(63);
    assert_eq!(id3.len() % 2, 1, "odd, so the chunk has a pad byte");
    let wav = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 16))
        .chunk_pad(b"id3 ", &id3, Some(0x55))
        .chunk(b"data", &[1, 0, 2, 0])
        .chunk(b"LIST", b"INFOx\0")
        .build();
    let (out, report) = render(&wav, "wav", &ours()).expect("rendered");
    assert!(report.tags_added);
    assert_eq!(report.tags_not_added, None);
    let summary = report.tag_edit.expect("edited");
    assert_eq!(
        (summary.replaced, summary.appended, summary.grew),
        (1, 1, false)
    );
    let fates: Vec<BlockFate> = report.blocks.iter().map(|b| b.fate).collect();
    assert_eq!(fates, [Replaced, Edited, Replaced, Carried]);
    let (tag_out, id) = payload(&out, 1);
    assert_eq!(&id, b"id3 ");
    let edits = id3::edits_from(&ours().tag_edits).expect("valid");
    assert_eq!(
        tag_out,
        id3::edit_tag(&id3, &edits).expect("editable").bytes
    );
    assert_eq!(tag_out.len(), id3.len(), "the padding held the frames");
    let pad_at = 20 + 16 + 8 + id3.len();
    assert_eq!(out[pad_at], 0, "a rewritten chunk's pad byte is 0");
    assert_eq!(payload(&out, 3), payload(&wav, 3));
}

#[test]
fn an_aiff_tag_without_room_grows_and_the_form_size_follows() {
    let id3 = small_tag(0);
    let aiff = Form::aiff()
        .chunk(b"COMM", &comm(1, 2, 16, 44_100, None))
        .chunk(b"ID3 ", &id3)
        .chunk(b"SSND", &ssnd(0, &[0, 1, 0, 2]))
        .build();
    let (out, report) = render(&aiff, "aiff", &ours()).expect("rendered");
    let summary = report.tag_edit.expect("edited");
    assert!(summary.grew);
    let (tag_out, id) = payload(&out, 1);
    assert_eq!(&id, b"ID3 ");
    assert!(tag_out.len() > id3.len() + crate::id3::GROWTH_PADDING_BYTES);
    let h = read_header(&mut Cursor::new(&out), Path::new("t")).expect("valid output");
    assert!(!h.table.truncated);
    assert_eq!(report.output_bytes, out.len() as u64);
}

fn same_as_without_edits(file: &[u8], ext: &str, req: &RenderRequest) -> RenderReport {
    let (plain, _) = render(file, ext, &RenderRequest::default()).expect("rendered");
    let (out, report) = render(file, ext, req).expect("rendered");
    assert_eq!(out, plain, "every chunk carried");
    assert!(!report.tags_added && report.tag_edit.is_none());
    assert!(report.blocks.iter().all(|b| b.fate != BlockFate::Edited));
    report
}

#[test]
fn no_tag_two_tags_or_an_unsafe_tag_are_carried_and_reported() {
    let base = || Form::riff().chunk(b"fmt ", &fmt_pcm(1, 44_100, 16));
    let data = [1, 0, 2, 0];
    let none = base().chunk(b"data", &data).build();
    let report = same_as_without_edits(&none, "wav", &ours());
    assert_eq!(report.tags_not_added, Some(NotEditable::NoTag));

    let two = base()
        .chunk(b"id3 ", &small_tag(8))
        .chunk(b"data", &data)
        .chunk(b"ID3 ", &small_tag(8))
        .build();
    let report = same_as_without_edits(&two, "wav", &ours());
    assert_eq!(
        report.tags_not_added,
        Some(NotEditable::SeveralTags { count: 2 })
    );

    let unsync = tag(4, 0x80, &[], &[text(4, *b"TIT2", 0, "A")], 8);
    let file = base().chunk(b"ID3 ", &unsync).chunk(b"data", &data).build();
    let report = same_as_without_edits(&file, "wav", &ours());
    assert_eq!(report.tags_not_added, Some(NotEditable::Unsynchronised));

    let junk = base()
        .chunk(b"id3 ", b"not a tag")
        .chunk(b"data", &data)
        .build();
    let report = same_as_without_edits(&junk, "wav", &ours());
    assert!(matches!(
        report.tags_not_added,
        Some(NotEditable::Malformed { .. })
    ));

    // No edits requested: nothing to report.
    let report = same_as_without_edits(&junk, "wav", &RenderRequest::default());
    assert_eq!(report.tags_not_added, None);
}

#[test]
fn an_invalid_edit_refuses_the_render() {
    let wav = Form::riff()
        .chunk(b"fmt ", &fmt_pcm(1, 44_100, 16))
        .chunk(b"data", &[1, 0])
        .build();
    for bad in [
        request(&[("APIC", "x")]),
        request(&[("TXXX:BPM", "1"), ("TXXX:bpm", "2")]),
    ] {
        let err = render(&wav, "wav", &bad).expect_err("refused");
        assert!(matches!(err, Error::InvalidArgument(_)), "{err}");
    }
}

#[test]
fn a_tag_over_the_cap_is_not_read() {
    let chunk = Chunk {
        id: *b"id3 ",
        header_offset: 12,
        size_field: u32::MAX,
        size_declared: MAX_TAG_BYTES + 1,
        payload: 20..20 + MAX_TAG_BYTES + 1,
        pad: None,
        pad_missing: false,
        beyond_container: false,
    };
    let edits = id3::edits_from(&ours().tag_edits).expect("valid");
    // An empty reader: reading anything would fail with an I/O error.
    let got =
        edit_chunk(&mut Cursor::new(Vec::new()), Path::new("t"), &chunk, &edits).expect("no read");
    assert_eq!(
        got,
        Err(NotEditable::TooLarge {
            bytes: MAX_TAG_BYTES + 1
        })
    );
}
