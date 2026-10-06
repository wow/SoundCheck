//! Throughput of TPDF dither and of the requantiser (gain + rounding, with dither at 16 bits)
//! on 60 s of 44.1 kHz stereo 24-bit samples.
#![allow(missing_docs)] // criterion_group! emits an undocumented public function

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use sc_core::{AudioSpec, testsig};
use sc_dsp::{Requantiser, SourceDepth, Tpdf};

fn samples_24bit() -> Vec<i32> {
    let buf = testsig::seeded_noise(AudioSpec::CD, 1, 0.5, 60.0);
    // Inside +/-0.5 of full scale, so the cast is exact after rounding.
    #[allow(clippy::cast_possible_truncation)]
    buf.data
        .iter()
        .map(|x| (f64::from(*x) * 8_388_608.0).round() as i32)
        .collect()
}

fn bench_dither(c: &mut Criterion) {
    let input = samples_24bit();
    let mut group = c.benchmark_group("dither");
    group.throughput(Throughput::Elements(input.len() as u64));
    group.bench_function("tpdf 60s stereo", |b| {
        let mut out = vec![0.0; input.len()];
        b.iter(|| {
            Tpdf::new(1).fill(black_box(&mut out));
        });
    });
    for (name, out_bits) in [
        ("requantise -3.2 dB 24-bit 60s stereo", 24),
        ("requantise -3.2 dB 16-bit tpdf 60s stereo", 16),
    ] {
        group.bench_function(name, |b| {
            let mut out = vec![0; 4096 * 2];
            b.iter(|| {
                let mut q = Requantiser::new(SourceDepth::Int { bits: 24 }, out_bits, -3.2, 1)
                    .expect("valid settings");
                for block in input.chunks(out.len()) {
                    q.push_int(black_box(block), &mut out)
                        .expect("integer source");
                }
                out[0]
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_dither);
criterion_main!(benches);
