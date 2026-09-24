# sc-analysis

Measurements over decoded audio: sample peak now; ITU-R BS.1770-5 loudness with the S-P95 statistic, true peak, LRA, beat and downbeat tracking, meter estimation and the static grid solver as they are implemented.
Invariants: pure functions over `&[f32]` plus an `AudioSpec`, `f64` accumulators, fixed chunk sizes, no I/O.
