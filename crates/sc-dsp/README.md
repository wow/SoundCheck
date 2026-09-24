# sc-dsp

Pure signal-processing blocks on interleaved `f32` slices: gain (dB and linear), later TPDF dither, biquads, the kick-band filter and onset envelopes. Streaming API (`push`/`finish`) with whole-buffer helpers on top.
Invariants: no I/O, no allocation inside `push`, accumulators in `f64`, deterministic for a given block size.
