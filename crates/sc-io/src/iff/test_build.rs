//! Hand-built RIFF/FORM files for the unit tests: chunks written in order with explicit pad
//! bytes, container sizes computed or forced.

/// A container being built.
pub struct Form {
    big: bool,
    magic: [u8; 4],
    type_id: [u8; 4],
    /// Bytes after the 12-byte header.
    pub body: Vec<u8>,
}

impl Form {
    /// `RIFF`/`WAVE`.
    pub fn riff() -> Self {
        Self {
            big: false,
            magic: *b"RIFF",
            type_id: *b"WAVE",
            body: Vec::new(),
        }
    }

    /// `FORM`/`AIFF`.
    pub fn aiff() -> Self {
        Self {
            big: true,
            magic: *b"FORM",
            type_id: *b"AIFF",
            body: Vec::new(),
        }
    }

    /// `FORM`/`AIFC`.
    pub fn aifc() -> Self {
        Self {
            type_id: *b"AIFC",
            ..Self::aiff()
        }
    }

    fn size_bytes(&self, size: u32) -> [u8; 4] {
        if self.big {
            size.to_be_bytes()
        } else {
            size.to_le_bytes()
        }
    }

    /// A chunk with a 0 pad byte after an odd payload.
    pub fn chunk(self, id: &[u8], payload: &[u8]) -> Self {
        self.chunk_pad(id, payload, Some(0))
    }

    /// A chunk; `pad` is written after an odd payload when given.
    pub fn chunk_pad(mut self, id: &[u8], payload: &[u8], pad: Option<u8>) -> Self {
        let size = u32::try_from(payload.len()).expect("test chunks are small");
        self.chunk_sized(id, size, payload);
        if payload.len() % 2 == 1
            && let Some(p) = pad
        {
            self.body.push(p);
        }
        self
    }

    /// A chunk header with a forced size field followed by `payload` (no pad).
    pub fn chunk_sized(&mut self, id: &[u8], size: u32, payload: &[u8]) {
        self.body.extend_from_slice(id);
        let size = self.size_bytes(size);
        self.body.extend_from_slice(&size);
        self.body.extend_from_slice(payload);
    }

    /// Raw bytes inside the container.
    pub fn raw(mut self, bytes: &[u8]) -> Self {
        self.body.extend_from_slice(bytes);
        self
    }

    /// The file with the container size counting exactly the body.
    pub fn build(&self) -> Vec<u8> {
        let size = u32::try_from(4 + self.body.len()).expect("test files are small");
        self.build_with_size(size)
    }

    /// The file with a forced container size field.
    pub fn build_with_size(&self, size: u32) -> Vec<u8> {
        let mut out = self.magic.to_vec();
        out.extend_from_slice(&self.size_bytes(size));
        out.extend_from_slice(&self.type_id);
        out.extend_from_slice(&self.body);
        out
    }
}

/// A `fmt ` payload: `tag`, channels, rate, block align and bits as given.
pub fn fmt(tag: u16, channels: u16, rate: u32, block_align: u16, bits: u16) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&tag.to_le_bytes());
    p.extend_from_slice(&channels.to_le_bytes());
    p.extend_from_slice(&rate.to_le_bytes());
    p.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
    p.extend_from_slice(&block_align.to_le_bytes());
    p.extend_from_slice(&bits.to_le_bytes());
    p
}

/// A plain PCM `fmt ` payload with the natural block align.
pub fn fmt_pcm(channels: u16, rate: u32, bits: u16) -> Vec<u8> {
    fmt(1, channels, rate, channels * bits.div_ceil(8), bits)
}

/// An extensible `fmt ` payload with sub-format `code` (1 PCM, 3 float).
pub fn fmt_extensible(channels: u16, rate: u32, bits: u16, valid: u16, code: u16) -> Vec<u8> {
    let mut p = fmt(0xFFFE, channels, rate, channels * bits / 8, bits);
    p.extend_from_slice(&22_u16.to_le_bytes());
    p.extend_from_slice(&valid.to_le_bytes());
    p.extend_from_slice(&3_u32.to_le_bytes());
    p.extend_from_slice(&u32::from(code).to_le_bytes());
    p.extend_from_slice(&[
        0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
    ]);
    p
}

/// Exact 80-bit extended encoding of a positive integer.
pub fn extended(rate: u32) -> [u8; 10] {
    let e = rate.ilog2();
    let exponent = u16::try_from(16_383 + e).expect("small exponent");
    let mantissa = u64::from(rate) << (63 - e);
    let mut out = [0_u8; 10];
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// A `COMM` payload; `compression` adds the AIFF-C type and an empty name.
pub fn comm(
    channels: u16,
    frames: u32,
    bits: u16,
    rate: u32,
    compression: Option<&[u8; 4]>,
) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&channels.to_be_bytes());
    p.extend_from_slice(&frames.to_be_bytes());
    p.extend_from_slice(&bits.to_be_bytes());
    p.extend_from_slice(&extended(rate));
    if let Some(c) = compression {
        p.extend_from_slice(c);
        p.extend_from_slice(&[0, 0]);
    }
    p
}

/// An `SSND` payload: offset, block size 0, `offset` filler bytes, then `audio`.
pub fn ssnd(offset: u32, audio: &[u8]) -> Vec<u8> {
    let mut p = offset.to_be_bytes().to_vec();
    p.extend_from_slice(&0_u32.to_be_bytes());
    p.extend(std::iter::repeat_n(0xEE, offset as usize));
    p.extend_from_slice(audio);
    p
}

/// Every integer sample of a file in memory and the count of samples with non-zero padding
/// bits.
pub fn read_ints(bytes: &[u8]) -> sc_core::Result<(Vec<i32>, u64)> {
    use std::io::Cursor;
    let path = std::path::Path::new("t");
    let format = super::read_header(&mut Cursor::new(bytes), path)?.format;
    read_ints_with(bytes, &format)
}

/// As [`read_ints`] with a given format.
pub fn read_ints_with(
    bytes: &[u8],
    format: &super::AudioFormat,
) -> sc_core::Result<(Vec<i32>, u64)> {
    let path = std::path::Path::new("t");
    let mut pcm = super::PcmReader::new(std::io::Cursor::new(bytes), format, path)?;
    let (mut all, mut block) = (Vec::new(), Vec::new());
    while pcm.next_block_int(&mut block)? > 0 {
        all.extend_from_slice(&block);
    }
    Ok((all, pcm.padding_bits_nonzero()))
}

/// Every float sample of a file in memory.
pub fn read_floats(bytes: &[u8]) -> sc_core::Result<Vec<f64>> {
    use std::io::Cursor;
    let path = std::path::Path::new("t");
    let format = super::read_header(&mut Cursor::new(bytes), path)?.format;
    let mut pcm = super::PcmReader::new(Cursor::new(bytes), &format, path)?;
    let (mut all, mut block) = (Vec::new(), Vec::new());
    while pcm.next_block_float(&mut block)? > 0 {
        all.extend_from_slice(&block);
    }
    Ok(all)
}
