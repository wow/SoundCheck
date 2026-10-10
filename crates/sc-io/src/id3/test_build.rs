//! Hand-built `ID3v2` tags for the unit tests: frames with any flags and text encoding, tags
//! with an extended header, padding or a footer.

use super::parse::encode_syncsafe;

/// Text in an `ID3v2` encoding: 0 ISO-8859-1, 1 UTF-16LE with a byte-order mark, 2 UTF-16BE,
/// 3 UTF-8; `terminated` adds the encoding's terminator.
pub fn encode(encoding: u8, text: &str, terminated: bool) -> Vec<u8> {
    let mut out = match encoding {
        0 => text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).expect("Latin-1 text"))
            .collect(),
        1 => [0xFF, 0xFE]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect(),
        2 => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        _ => text.as_bytes().to_vec(),
    };
    if terminated {
        out.extend(if encoding == 1 || encoding == 2 {
            &[0, 0][..]
        } else {
            &[0][..]
        });
    }
    out
}

/// A frame of a tag of `major`: id, size (plain in v2.3, syncsafe in v2.4), flags, body.
pub fn frame(major: u8, id: [u8; 4], flags: [u8; 2], body: &[u8]) -> Vec<u8> {
    let len = u32::try_from(body.len()).expect("small frames");
    let mut out = id.to_vec();
    if major == 3 {
        out.extend_from_slice(&len.to_be_bytes());
    } else {
        out.extend_from_slice(&encode_syncsafe(len).expect("small frames"));
    }
    out.extend_from_slice(&flags);
    out.extend_from_slice(body);
    out
}

/// A text frame in `encoding`.
pub fn text(major: u8, id: [u8; 4], encoding: u8, value: &str) -> Vec<u8> {
    let mut body = vec![encoding];
    body.extend(encode(encoding, value, false));
    frame(major, id, [0, 0], &body)
}

/// A `TXXX` frame in `encoding`.
pub fn txxx(major: u8, encoding: u8, desc: &str, value: &str) -> Vec<u8> {
    let mut body = vec![encoding];
    body.extend(encode(encoding, desc, true));
    body.extend(encode(encoding, value, false));
    frame(major, *b"TXXX", [0, 0], &body)
}

/// A `GEOB` body in `encoding`: MIME type (ISO-8859-1), file name, description, object.
pub fn geob_body(encoding: u8, mime: &str, file_name: &str, desc: &str, object: &[u8]) -> Vec<u8> {
    let mut body = vec![encoding];
    body.extend(encode(0, mime, true));
    body.extend(encode(encoding, file_name, true));
    body.extend(encode(encoding, desc, true));
    body.extend_from_slice(object);
    body
}

/// A `GEOB` frame shaped like Serato's: ISO-8859-1, `application/octet-stream`, no file name.
pub fn geob(major: u8, desc: &str, object: &[u8]) -> Vec<u8> {
    let body = geob_body(0, "application/octet-stream", "", desc, object);
    frame(major, *b"GEOB", [0, 0], &body)
}

/// A `COMM` body in `encoding`: language, short description, text.
pub fn comm_body(encoding: u8, language: [u8; 3], desc: &str, value: &str) -> Vec<u8> {
    let mut body = vec![encoding];
    body.extend_from_slice(&language);
    body.extend(encode(encoding, desc, true));
    body.extend(encode(encoding, value, false));
    body
}

/// A `COMM` frame in `encoding`, language `eng`.
pub fn comm(major: u8, encoding: u8, desc: &str, value: &str) -> Vec<u8> {
    frame(
        major,
        *b"COMM",
        [0, 0],
        &comm_body(encoding, *b"eng", desc, value),
    )
}

/// `body` with a zero byte after every 0xFF (ID3v2.4 frame-level unsynchronisation).
pub fn unsynchronise(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() * 2);
    for &b in body {
        out.push(b);
        if b == 0xFF {
            out.push(0);
        }
    }
    out
}

/// Header (`ID3`, `major`, revision 0, `flags`), extended header, frames, `padding` zero bytes.
pub fn tag(major: u8, flags: u8, ext: &[u8], frames: &[Vec<u8>], padding: usize) -> Vec<u8> {
    let body: Vec<u8> = frames.concat();
    let size = u32::try_from(ext.len() + body.len() + padding).expect("small tags");
    let mut out = b"ID3".to_vec();
    out.extend_from_slice(&[major, 0, flags]);
    out.extend_from_slice(&encode_syncsafe(size).expect("small tags"));
    out.extend_from_slice(ext);
    out.extend_from_slice(&body);
    out.resize(out.len() + padding, 0);
    out
}

/// A v2.4 tag with a footer (flag 0x10) and no padding.
pub fn tag_with_footer(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = tag(4, 0x10, &[], frames, 0);
    let mut footer = out[..10].to_vec();
    footer[..3].copy_from_slice(b"3DI");
    out.extend(footer);
    out
}

/// A v2.3 extended header without a CRC holding `padding` as its padding size.
pub fn ext_v23(padding: u32) -> Vec<u8> {
    [&[0, 0, 0, 6, 0, 0][..], &padding.to_be_bytes()].concat()
}

/// A v2.4 extended header with the "tag is an update" flag (no flag data).
pub fn ext_v24_update() -> Vec<u8> {
    vec![0, 0, 0, 6, 1, 0x40]
}
