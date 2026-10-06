//! Throughput of the ID3 edit: a v2.3 tag of about 2 MiB (a 2 MiB `APIC` cover, 40 text and
//! `TXXX` frames, three Serato-style `GEOB` objects, 1 KiB of padding) indexed and rebuilt
//! with SoundCheck's five frames (`TBPM` replaced in place, the rest appended), in memory.
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use sc_io::id3::{Edit, edit_tag};

fn frame(id: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend_from_slice(&u32::try_from(body.len()).expect("small").to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(body);
    out
}

fn syncsafe(v: usize) -> [u8; 4] {
    let v = u32::try_from(v).expect("small");
    [
        (v >> 21) & 0x7F,
        (v >> 14) & 0x7F,
        (v >> 7) & 0x7F,
        v & 0x7F,
    ]
    .map(|b| u8::try_from(b).expect("seven bits"))
}

fn tag() -> Vec<u8> {
    let mut frames = Vec::new();
    frames.extend(frame(*b"TBPM", b"\x00127"));
    for i in 0..20 {
        frames.extend(frame(*b"TIT2", format!("\x00Title {i}").as_bytes()));
        frames.extend(frame(
            *b"TXXX",
            format!("\x00FIELD{i}\x00value {i}").as_bytes(),
        ));
    }
    for desc in ["Serato Autotags", "Serato Markers2", "Serato BeatGrid"] {
        let mut body = b"\x00application/octet-stream\x00\x00".to_vec();
        body.extend_from_slice(desc.as_bytes());
        body.push(0);
        body.extend(std::iter::repeat_n(0x5A, 470));
        frames.extend(frame(*b"GEOB", &body));
    }
    let mut apic = b"\x00image/jpeg\x00\x03\x00".to_vec();
    apic.extend((0..2 << 20).map(|i: u32| i.wrapping_mul(2_654_435_761).to_be_bytes()[0]));
    frames.extend(frame(*b"APIC", &apic));
    let padding = 1024;
    let mut out = b"ID3\x03\x00\x00".to_vec();
    out.extend_from_slice(&syncsafe(frames.len() + padding));
    out.extend(frames);
    out.resize(out.len() + padding, 0);
    out
}

fn bench(c: &mut Criterion) {
    let tag = tag();
    let edits: Vec<Edit> = [
        ("TBPM", "128"),
        ("TXXX:BPM", "128.00"),
        ("TXXX:REPLAYGAIN_TRACK_GAIN", "-6.20 dB"),
        ("TXXX:REPLAYGAIN_TRACK_PEAK", "0.912345"),
        ("TXXX:SOUNDCHECK", &"{\"schema\":1}".repeat(20)),
    ]
    .iter()
    .map(|(l, v)| Edit::new(l, v).expect("valid edit"))
    .collect();
    let mut group = c.benchmark_group("id3");
    group.throughput(Throughput::Bytes(tag.len() as u64));
    group.bench_function("edit_2mib_tag", |b| {
        b.iter(|| edit_tag(std::hint::black_box(&tag), &edits).expect("editable"));
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
