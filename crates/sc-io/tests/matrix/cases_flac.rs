//! FLAC fixtures of the matrix (rules in `cases.rs`).

use super::cases::{FRAMES, Fixture, Symphonia, finish, spec};
use super::flac::{self, Meta, Wrap};
use super::id3::{self, Version};
use super::parse::Container;
use super::pcm;
use super::riff;

/// Vorbis `SERATO_*` values: base64 of the GEOB envelope (MIME, empty file name, description)
/// followed by the object.
fn serato_field(name: &str, desc: &str, object: &[u8]) -> String {
    let mut envelope = b"application/octet-stream\0\0".to_vec();
    envelope.extend_from_slice(desc.as_bytes());
    envelope.push(0);
    envelope.extend_from_slice(object);
    format!("{name}={}", id3::base64(&envelope))
}

fn vorbis(fields: &[&str], serato: bool) -> Meta {
    let mut fields: Vec<String> = fields.iter().map(ToString::to_string).collect();
    if serato {
        fields.push(serato_field(
            "SERATO_MARKERS_V2",
            "Serato Markers2",
            &id3::serato_markers2(1_875),
        ));
        fields.push(serato_field(
            "SERATO_BEATGRID",
            "Serato BeatGrid",
            &id3::serato_beatgrid(0.012, 128.0),
        ));
    }
    Meta::Vorbis {
        vendor: "reference libFLAC 1.4.3 20230623".into(),
        fields,
    }
}

fn png() -> Meta {
    Meta::Picture {
        mime: "image/png".into(),
        data: id3::PNG_1X1.to_vec(),
    }
}

fn build(name: &'static str, src: &pcm::Samples, rate: u32, ch: u16, metas: &[Meta]) -> Fixture {
    build_wrapped(name, src, rate, ch, metas, &Wrap::default())
}

fn build_wrapped(
    name: &'static str,
    src: &pcm::Samples,
    rate: u32,
    ch: u16,
    metas: &[Meta],
    wrap: &Wrap,
) -> Fixture {
    finish(
        spec(name, Container::Flac, rate, ch, src.clone()),
        flac::build(src, rate, ch, metas, wrap),
    )
}

fn flac16_all_blocks() -> Fixture {
    let src = pcm::int_samples(44_100, 2, FRAMES, 16, 11);
    // A foreign RIFF chunk as `flac --keep-foreign-metadata` stores it.
    let list = riff::list_info(&[(b"INAM", "Matrix Tone")]).payload;
    let mut foreign = b"LIST".to_vec();
    foreign.extend_from_slice(&u32::try_from(list.len()).expect("short").to_le_bytes());
    foreign.extend_from_slice(&list);
    let metas = [
        Meta::SeekTable {
            every: 2,
            placeholders: 2,
        },
        vorbis(
            &[
                "TITLE=Matrix Tone",
                "ARTIST=SoundCheck Ensemble",
                "ARTIST=Second Artist",
                "BPM=128",
                "REPLAYGAIN_TRACK_GAIN=-4.10 dB",
                "comment=lower-case name, UTF-8 caf\u{e9}",
                "replaygain_track_peak=0.700000",
            ],
            true,
        ),
        Meta::Cuesheet,
        Meta::Application {
            id: *b"riff",
            data: foreign,
        },
        png(),
        Meta::Unknown {
            ty: 100,
            data: pcm::XorShift::new(12).bytes(37),
        },
        Meta::Padding(1024),
    ];
    build("flac16-all-blocks", &src, 44_100, 2, &metas)
}

fn flac24_minimal() -> Fixture {
    let src = pcm::int_samples(48_000, 2, FRAMES, 24, 13);
    let metas = [
        vorbis(&["TITLE=Matrix Tone", "BPM=128.00"], false),
        Meta::Padding(512),
    ];
    build("flac24-48k-minimal", &src, 48_000, 2, &metas)
}

fn flac16_mono() -> Fixture {
    let src = pcm::int_samples(44_100, 1, FRAMES, 16, 15);
    let metas = [vorbis(&["TITLE=Mono"], false), png(), Meta::Padding(256)];
    build("flac16-mono", &src, 44_100, 1, &metas)
}

/// Some taggers put an `ID3v2` tag in front of `fLaC` and an `ID3v1` tag at the end; both are
/// carried as blobs. libFLAC (`flac -t`) decodes every sample of such a file but warns about
/// the `ID3v2` tag and reports lost sync at the `ID3v1` tag, so a `flac -t` check on outputs has to
/// allow what the input already carried. The Vorbis block already holds a `SOUNDCHECK` field from an earlier run,
/// which an edit replaces in place.
fn flac16_id3_wrapped() -> Fixture {
    let src = pcm::int_samples(44_100, 1, FRAMES, 16, 23);
    let leading = id3::tag(Version::V23, &id3::frames(Version::V23, false, None), 32).bytes;
    let wrap = Wrap {
        leading,
        trailing: riff::id3v1("Matrix Tone", "SoundCheck Ensemble"),
    };
    let metas = [
        Meta::SeekTable {
            every: 1,
            placeholders: 1,
        },
        vorbis(
            &[
                "TITLE=Matrix Tone",
                r#"SOUNDCHECK={"schema":1,"gain_db":-1.0}"#,
                "GENRE=Techno",
            ],
            false,
        ),
        Meta::Padding(128),
    ];
    let mut fx = build_wrapped("flac16-mono-id3v2-id3v1", &src, 44_100, 1, &metas, &wrap);
    fx.symphonia = Symphonia::Truncates {
        frames: 4 * flac::BLOCK_SIZE,
        why: "symphonia 0.6.1 reads a trailing ID3v1 tag as part of the last FLAC frame and \
              drops that frame; sc-io must cut the tag off before decoding",
    };
    fx
}

/// The FLAC fixtures.
#[must_use]
pub fn flac() -> Vec<Fixture> {
    vec![
        flac16_all_blocks(),
        flac24_minimal(),
        flac16_mono(),
        flac16_id3_wrapped(),
    ]
}
