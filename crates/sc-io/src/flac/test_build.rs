//! FLAC streams for unit tests, encoded with `flac-codec`'s frame writer directly (not through
//! [`super::FrameEncoder`]) and assembled with metadata blocks given as `(type, payload)`.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use flac_codec::encode::{FlacStreamWriter, Options};
use md5::{Digest, Md5};

use super::{MARKER, StreamInfo, block_header, le_sample_bytes};

/// Deterministic interleaved samples at `bits` bits: a slow ramp plus xorshift noise, within
/// about half of full scale.
pub(crate) fn samples(frames: usize, channels: u16, bits: u8, seed: u32) -> Vec<i32> {
    let mut x = seed.max(1);
    let quarter = 1_i64 << (bits - 2);
    (0..frames * usize::from(channels))
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let ramp = i64::try_from(i % 4000).expect("small") * quarter / 4000 - quarter / 2;
            let noise = i64::from(x.cast_signed()) >> (34 - u32::from(bits));
            i32::try_from(ramp + noise).expect("fits the depth")
        })
        .collect()
}

/// A writer the test keeps a handle on.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Frames of 4096 samples (the last shorter) and their STREAMINFO.
pub(crate) fn encode(data: &[i32], rate: u32, channels: u16, bits: u8) -> (Vec<u8>, StreamInfo) {
    let ch = usize::from(channels);
    let shared = Shared::default();
    let mut w = FlacStreamWriter::new(shared.clone(), Options::default());
    let (mut min, mut max, mut before) = (usize::MAX, 0, 0);
    for block in data.chunks(4096 * ch) {
        w.write(rate, u8::try_from(channels).expect("1..8"), u32::from(bits), block)
            .expect("encode");
        let len = shared.0.borrow().len() - before;
        (min, max) = (min.min(len), max.max(len));
        before += len;
    }
    let out = shared.0.borrow().clone();
    let mut md5 = Md5::new();
    let mut bytes = Vec::new();
    le_sample_bytes(data, usize::from(bits).div_ceil(8), &mut bytes);
    md5.update(&bytes);
    let info = StreamInfo {
        min_block: 4096,
        max_block: 4096,
        min_frame: u32::try_from(min).expect("small"),
        max_frame: u32::try_from(max).expect("small"),
        sample_rate_hz: rate,
        channels: u8::try_from(channels).expect("1..8"),
        bits,
        total_samples: (data.len() / ch) as u64,
        md5: md5.finalize().into(),
    };
    (out, info)
}

/// A FLAC file: `leading`, `fLaC`, STREAMINFO, `blocks` (last flag on the final one), the
/// frames, `trailing`.
pub(crate) fn file(
    leading: &[u8],
    info: &StreamInfo,
    blocks: &[(u8, Vec<u8>)],
    frames: &[u8],
    trailing: &[u8],
) -> Vec<u8> {
    let mut f = leading.to_vec();
    f.write_all(&MARKER).expect("vec");
    f.extend_from_slice(&block_header(blocks.is_empty(), 0, 34));
    f.extend_from_slice(&info.to_bytes());
    for (i, (ty, payload)) in blocks.iter().enumerate() {
        let len = u32::try_from(payload.len()).expect("small");
        f.extend_from_slice(&block_header(i + 1 == blocks.len(), *ty, len));
        f.extend_from_slice(payload);
    }
    f.extend_from_slice(frames);
    f.extend_from_slice(trailing);
    f
}

/// A `VORBIS_COMMENT` payload.
pub(crate) fn vorbis(vendor: &str, fields: &[&str]) -> Vec<u8> {
    let len = |n: usize| u32::try_from(n).expect("short").to_le_bytes();
    let mut p = len(vendor.len()).to_vec();
    p.extend_from_slice(vendor.as_bytes());
    p.extend_from_slice(&len(fields.len()));
    for f in fields {
        p.extend_from_slice(&len(f.len()));
        p.extend_from_slice(f.as_bytes());
    }
    p
}

/// An `ID3v1` tag.
pub(crate) fn id3v1() -> Vec<u8> {
    let mut t = b"TAG".to_vec();
    t.resize(128, b' ');
    t
}

/// A minimal `ID3v2.3` tag with `padding` zero bytes.
pub(crate) fn id3v2(padding: usize) -> Vec<u8> {
    let mut t = b"ID3\x03\x00\x00".to_vec();
    let size = u32::try_from(padding).expect("small");
    t.extend_from_slice(&super::super::id3::encode_syncsafe(size).expect("small"));
    t.resize(10 + padding, 0);
    t
}
