//! What a plan did, in words: the grid report's notes, the command line's lines and the row
//! notes the app shows. [`skip_why`] is only the "why", since what to do about it depends on where
//! it is shown; [`skip_note`] is the app's row.

use sc_core::export::{ExportNotice, ExportSkip, XmlOnlyReason};

/// The note on a written file whose grid needs review and was not confirmed.
pub const GRID_WITHHELD: &str =
    "its BPM and bar 1 are not written, because the grid needs review (confirm it in the app)";

/// The note on an MP3 or AAC file the XML carries.
pub const UNVERIFIED: &str = "in the rekordbox XML its bar 1 is not yet verified \
    against the encoder delay rekordbox applies to MP3 and AAC";

/// What the user should know about a written file.
#[must_use]
pub fn notice_text(notice: ExportNotice) -> String {
    match notice {
        ExportNotice::SeratoCuesShifted { cut_s } => format!(
            "the copy keeps its Serato cue points and beat grid as they are, so in Serato they sit \
             {:.2} s late; the original is unchanged",
            cut_s.0
        ),
        ExportNotice::SeratoCuesMayHaveShifted { cut_s } => format!(
            "its tags could not be read to rule out Serato data; if it holds any, the copy keeps \
             its cue points and beat grid as they are, so in Serato they may have shifted and sit \
             {:.2} s late; the original is unchanged",
            cut_s.0
        ),
    }
}

/// Why only the rekordbox XML carries a file's grid.
#[must_use]
pub fn xml_only_text(reason: XmlOnlyReason) -> String {
    match reason {
        XmlOnlyReason::Mp3OrAac { codec } => {
            format!("{} (file writes arrive later)", codec.label())
        }
        XmlOnlyReason::GridOnlyFlac => {
            "grid only on FLAC (writing tags alone to FLAC arrives later)".to_owned()
        }
        XmlOnlyReason::GridOnlyWouldRequantise { float, bits } => {
            let bits = bits.map_or_else(String::new, |b| format!("{b}-bit "));
            let kind = if float { "float" } else { "integer" };
            format!("grid only would change the samples of this {bits}{kind} file")
        }
        XmlOnlyReason::NoTagToWriteGridOnly => {
            "grid only, and the file has no tag to write into".to_owned()
        }
    }
}

/// A skipped row's note in the app: why, with what to do when no other part of the app says it.
#[must_use]
pub fn skip_note(reason: ExportSkip) -> String {
    let why = skip_why(reason);
    match reason {
        ExportSkip::CopyOfOtherTrack | ExportSkip::SameNameInBatch => {
            format!("{why}; rename one, or choose another folder")
        }
        _ => why,
    }
}

/// Why a file is skipped.
#[must_use]
pub fn skip_why(reason: ExportSkip) -> String {
    match reason {
        ExportSkip::SeratoInPlaceCut { cut_s } => format!(
            "cutting {:.2} s in place would move its Serato cue points, which are stored as \
             positions in the audio",
            cut_s.0
        ),
        ExportSkip::SeratoUnknownInPlaceCut { cut_s } => format!(
            "its tags could not be read to rule out Serato data, and cutting {:.2} s in place \
             would move Serato cue points if it has any",
            cut_s.0
        ),
        ExportSkip::NotDjSafeRate { sample_rate_hz } => format!(
            "its sample rate, {sample_rate_hz} Hz, is not one DJ players accept (44.1 or 48 kHz)"
        ),
        ExportSkip::UnsupportedChannels { channels } => {
            format!(
                "it has {channels} channels; DJ players and SoundCheck write mono or stereo only"
            )
        }
        ExportSkip::NothingToWrite { reason } => format!(
            "only the rekordbox XML could carry its grid ({}), and the XML is off",
            xml_only_text(reason)
        ),
        ExportSkip::Unsupported { codec } => format!(
            "{} files are analysed only; SoundCheck does not write them",
            codec.label()
        ),
        ExportSkip::Silent => {
            "it has no audio above the -70 LUFS gate, so there is no loudness to align".to_owned()
        }
        ExportSkip::NoGrid => {
            "grid only, and no beats were found, so there is no grid to write".to_owned()
        }
        ExportSkip::XmlNoGrid { codec } => format!(
            "{} files are carried by the rekordbox XML alone, and no beats were found, so it \
             would carry nothing",
            codec.label()
        ),
        ExportSkip::XmlGridNeedsReview { codec } => format!(
            "{} files are carried by the rekordbox XML alone, and its grid needs review, so it \
             is not written and the XML would carry nothing",
            codec.label()
        ),
        ExportSkip::XmlNotOptedIn { codec } => format!(
            "{} files are carried by the rekordbox XML alone, and in Library mode the XML lists \
             a track only once its grid is confirmed (importing it replaces the DJ app's own \
             grid)",
            codec.label()
        ),
        ExportSkip::OtherFileInFolder => {
            "a file of this name is already in the folder, and it is not a copy SoundCheck wrote \
             (or it changed since), so it is left as it is"
                .to_owned()
        }
        ExportSkip::CopyOfOtherTrack => {
            "the folder has a copy of this name that SoundCheck made from another file, or from \
             this one before it was moved and changed"
                .to_owned()
        }
        ExportSkip::SameNameInBatch => {
            "a file earlier in this export takes this name in the folder".to_owned()
        }
        ExportSkip::GridNeedsReview => {
            "grid only, and its grid needs review, so it is not written and there is nothing \
             else to write"
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests;
