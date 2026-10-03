//! Deterministic test audio for the fixture matrix.
//!
//! Two partials per channel plus seeded xorshift noise, peaking near -10 dBFS (RMS about
//! -16 dBFS). Only `+`, `-`, `*` and `/` are used (the sine is a Taylor series after range
//! reduction), so the samples are bit-identical on every platform and the golden manifest does
//! not depend on the system maths library.

/// Source samples of a fixture, interleaved, exactly as they are stored in the file.
#[derive(Debug, Clone, PartialEq)]
pub enum Samples {
    /// Signed integer PCM; each value fits in `bits` bits (two's complement).
    Int {
        /// Bits per sample: 16 or 24.
        bits: u8,
        /// Interleaved sample values.
        data: Vec<i32>,
    },
    /// IEEE-754 single precision, nominal range -1.0..1.0.
    Float(Vec<f32>),
}

impl Samples {
    /// Number of interleaved values.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Int { data, .. } => data.len(),
            Self::Float(data) => data.len(),
        }
    }

    /// Stored bits per sample (32 for float).
    #[must_use]
    pub fn bits(&self) -> u8 {
        match self {
            Self::Int { bits, .. } => *bits,
            Self::Float(_) => 32,
        }
    }

    /// Largest absolute value as a fraction of full scale.
    #[must_use]
    pub fn peak(&self) -> f64 {
        (0..self.len())
            .map(|i| self.normalised(i).abs())
            .fold(0.0, f64::max)
    }

    /// Value `i` as a fraction of full scale (integers divided by `2^(bits-1)`).
    #[must_use]
    pub fn normalised(&self, i: usize) -> f64 {
        match self {
            Self::Int { bits, data } => f64::from(data[i]) / f64::from(1_u32 << (bits - 1)),
            Self::Float(data) => f64::from(data[i]),
        }
    }
}

/// Seeded xorshift64* generator (Vigna 2016); never seeded from time.
pub struct XorShift(u64);

impl XorShift {
    /// A generator for `seed` (0 is mapped to a fixed non-zero state).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in -1.0..1.0 from the top 53 bits.
    pub fn next_bipolar(&mut self) -> f64 {
        // 53 bits fit an f64 mantissa exactly.
        #[allow(clippy::cast_precision_loss)]
        let unit = (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64;
        2.0 * unit - 1.0
    }

    /// `n` random bytes.
    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next_u64().to_be_bytes()[0]).collect()
    }
}

/// Sine from basic arithmetic only: reduce to -pi/2..pi/2, then a Taylor series to x^19
/// (truncation error below 1e-12 there).
#[must_use]
pub fn det_sin(x: f64) -> f64 {
    let tau = 2.0 * std::f64::consts::PI;
    let mut r = x - tau * (x / tau).floor(); // 0..tau
    if r > std::f64::consts::PI {
        r -= tau; // -pi..pi
    }
    let half_pi = std::f64::consts::FRAC_PI_2;
    if r > half_pi {
        r = std::f64::consts::PI - r;
    } else if r < -half_pi {
        r = -std::f64::consts::PI - r;
    }
    let r2 = r * r;
    let mut term = r;
    let mut sum = r;
    for k in 1..10_u32 {
        let k = f64::from(k);
        term = -term * r2 / ((2.0 * k) * (2.0 * k + 1.0));
        sum += term;
    }
    sum
}

/// Interleaved test signal in full-scale units: channel `c` holds partials at
/// `220 * (1 + c/2)` Hz (0.2) and `1250 + 90 c` Hz (0.08) plus uniform noise (0.04).
#[must_use]
pub fn tone(sample_rate: u32, channels: u16, frames: usize, seed: u64) -> Vec<f64> {
    let mut rng = XorShift::new(seed);
    let rate = f64::from(sample_rate);
    let mut out = Vec::with_capacity(frames * usize::from(channels));
    for n in 0..frames {
        // Frame counts stay far below 2^52, so the conversion is exact.
        #[allow(clippy::cast_precision_loss)]
        let t = n as f64 / rate;
        for c in 0..channels {
            let c = f64::from(c);
            let tau = 2.0 * std::f64::consts::PI;
            let low = 0.2 * det_sin(tau * 220.0 * (1.0 + 0.5 * c) * t);
            let high = 0.08 * det_sin(tau * (1250.0 + 90.0 * c) * t + c);
            out.push(low + high + 0.04 * rng.next_bipolar());
        }
    }
    out
}

/// Rounds full-scale values to `bits`-bit integers (clamped to the representable range).
#[must_use]
pub fn quantise(signal: &[f64], bits: u8) -> Vec<i32> {
    let scale = f64::from(1_u32 << (bits - 1));
    let (lo, hi) = (-scale, scale - 1.0);
    signal
        .iter()
        // Clamped to the bits-bit range first, so the cast cannot truncate.
        .map(|x| {
            #[allow(clippy::cast_possible_truncation)]
            let q = (x * scale).round().clamp(lo, hi) as i32;
            q
        })
        .collect()
}

/// Rounds full-scale values to the nearest `f32`.
#[must_use]
pub fn to_f32(signal: &[f64]) -> Vec<f32> {
    // Rounding to nearest f32 is the intent.
    #[allow(clippy::cast_possible_truncation)]
    signal.iter().map(|x| *x as f32).collect()
}

/// Integer samples for a fixture.
#[must_use]
pub fn int_samples(sample_rate: u32, channels: u16, frames: usize, bits: u8, seed: u64) -> Samples {
    Samples::Int {
        bits,
        data: quantise(&tone(sample_rate, channels, frames, seed), bits),
    }
}

/// Float samples scaled by `factor` (a "hot" float master can exceed full scale).
#[must_use]
pub fn float_samples_scaled(
    sample_rate: u32,
    channels: u16,
    frames: usize,
    seed: u64,
    factor: f64,
) -> Samples {
    let signal: Vec<f64> = tone(sample_rate, channels, frames, seed)
        .iter()
        .map(|x| x * factor)
        .collect();
    Samples::Float(to_f32(&signal))
}

/// Little-endian bytes as stored in a WAV `data` chunk or an AIFF-C `sowt` `SSND`.
#[must_use]
pub fn bytes_le(samples: &Samples) -> Vec<u8> {
    match samples {
        Samples::Int { bits, data } => {
            let width = usize::from(*bits / 8);
            data.iter()
                .flat_map(|s| s.to_le_bytes().into_iter().take(width))
                .collect()
        }
        Samples::Float(data) => data.iter().flat_map(|s| s.to_le_bytes()).collect(),
    }
}

/// Big-endian bytes as stored in an AIFF `SSND` chunk.
#[must_use]
pub fn bytes_be(samples: &Samples) -> Vec<u8> {
    match samples {
        Samples::Int { bits, data } => {
            let width = usize::from(*bits / 8);
            data.iter()
                .flat_map(|s| s.to_be_bytes().into_iter().skip(4 - width))
                .collect()
        }
        Samples::Float(data) => data.iter().flat_map(|s| s.to_be_bytes()).collect(),
    }
}
