//! What a file is before it is decoded: codec, sample rate, depth, channels, bitrate, duration
//! and the title, artist and album tags, read with lofty (read-only, cover art skipped), so a
//! dropped file shows its row at once. Anything unreadable leaves the field empty; the decoder
//! is the authority once analysis runs.

use std::fs::File;
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::iff::wav::{WavFile, WavFormat};
use lofty::mp4::{Mp4Codec, Mp4File};
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use sc_core::Seconds;
use sc_core::ipc::{DjUnsafe, FileInfo};
use sc_core::plan::Codec;

/// Reads `path`'s headers and tags; never fails, an unreadable file gives the codec its name
/// suggests and nothing else.
#[must_use]
pub fn probe(path: &Path) -> FileInfo {
    let mut info = FileInfo {
        codec: Codec::from_path(path),
        ..FileInfo::default()
    };
    let options = ParseOptions::new().read_cover_art(false);
    let Ok(tagged) = Probe::open(path).and_then(|p| p.options(options).read()) else {
        return info;
    };
    let file_type = tagged.file_type();
    info.codec = match file_type {
        FileType::Wav => Codec::Wav,
        FileType::Aiff => Codec::Aiff,
        FileType::Flac => Codec::Flac,
        FileType::Mpeg => Codec::Mp3,
        FileType::Aac => Codec::Aac,
        FileType::Mp4 => mp4_codec(path, options),
        FileType::Vorbis => Codec::Vorbis,
        FileType::Opus => Codec::Opus,
        _ => Codec::Other,
    };
    let props = tagged.properties();
    info.sample_rate = props.sample_rate();
    info.channels = props.channels();
    info.duration = Some(Seconds(props.duration().as_secs_f64()));
    let lossless = matches!(
        info.codec,
        Codec::Wav | Codec::Aiff | Codec::Flac | Codec::Alac
    );
    if lossless {
        info.bits_per_sample = props.bit_depth();
    } else {
        info.bitrate_kbps = props.audio_bitrate().map(|kbps| {
            if info.codec == Codec::Mp3 {
                nominal_mp3_bitrate(kbps)
            } else {
                kbps
            }
        });
    }
    if file_type == FileType::Wav {
        info.float = wav_is_float(path, options);
    }
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        let text = |key: ItemKey| {
            tag.get_string(key)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        info.title = text(ItemKey::TrackTitle);
        info.artist = text(ItemKey::TrackArtist);
        info.album = text(ItemKey::AlbumTitle);
    }
    info.dj_unsafe = dj_unsafe(&info);
    info
}

/// The first way `info`'s format is not DJ-safe (44.1 or 48 kHz, 16- or 24-bit integer PCM,
/// stereo); `None` for codecs that are analysed only, which export converts anyway.
#[must_use]
pub fn dj_unsafe(info: &FileInfo) -> Option<DjUnsafe> {
    if !info.codec.is_writable() {
        return None;
    }
    if info.sample_rate.is_some_and(|r| r != 44_100 && r != 48_000) {
        return Some(DjUnsafe::SampleRate);
    }
    if info.float {
        return Some(DjUnsafe::Float);
    }
    if info.bits_per_sample.is_some_and(|b| b != 16 && b != 24) {
        return Some(DjUnsafe::BitDepth);
    }
    if info.channels.is_some_and(|c| c != 2) {
        return Some(DjUnsafe::Channels);
    }
    None
}

/// MPEG-1 Layer III bitrates, kbit/s.
const MP3_BITRATES: [u32; 14] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];

/// lofty averages the bitrate over the whole file, headers included, so a 320 kbit/s CBR file
/// can read 321. Within 2 % of a standard rate, show the standard rate; a VBR average stays.
fn nominal_mp3_bitrate(kbps: u32) -> u32 {
    MP3_BITRATES
        .into_iter()
        .find(|&r| kbps.abs_diff(r) * 50 <= r)
        .unwrap_or(kbps)
}

fn mp4_codec(path: &Path, options: ParseOptions) -> Codec {
    let Ok(mut file) = File::open(path) else {
        return Codec::Aac;
    };
    match Mp4File::read_from(&mut file, options).map(|f| f.properties().codec()) {
        Ok(Some(Mp4Codec::ALAC)) => Codec::Alac,
        Ok(Some(Mp4Codec::MP3)) => Codec::Mp3,
        Ok(Some(Mp4Codec::FLAC)) => Codec::Flac,
        _ => Codec::Aac,
    }
}

fn wav_is_float(path: &Path, options: ParseOptions) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    WavFile::read_from(&mut file, options)
        .is_ok_and(|f| matches!(f.properties().format(), WavFormat::IEEE_FLOAT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_bitrates_snap_only_near_a_standard_rate() {
        assert_eq!(nominal_mp3_bitrate(129), 128);
        assert_eq!(nominal_mp3_bitrate(321), 320);
        assert_eq!(nominal_mp3_bitrate(245), 245);
        assert_eq!(nominal_mp3_bitrate(190), 192);
        assert_eq!(nominal_mp3_bitrate(185), 185);
    }
}
