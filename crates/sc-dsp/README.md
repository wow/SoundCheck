# sc-dsp

Pure signal-processing blocks on interleaved `f32` slices: gain (dB and linear), TPDF dither (`Tpdf`, seeded xorshift64*, triangular over (-1, 1) LSB), the `Requantiser` that applies a gain (factor from `db_to_linear`, the pure-Rust `libm` `pow`, so the same bits on every platform) and converts integer or float samples to 16/24-bit integers (exact shift at 0 dB, round half to even, TPDF only at 16 bits, saturation counted), biquads, the kick-band filter and onset envelopes. Streaming API (`push`/`finish`) with whole-buffer helpers on top.
Invariants: no I/O, no allocation inside `push`, accumulators in `f64`, deterministic for a given block size. Benches: `benches/gain.rs`, `benches/onset.rs`, `benches/dither.rs` (TPDF fill and requantisation of 60 s of stereo).
