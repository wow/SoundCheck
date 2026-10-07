//! Unit tests of `tags.rs`: the mapping per container and every refusal.

use super::*;

fn tag(text: &str) -> Tag {
    Tag::parse(text).expect("parsed")
}

fn labels(edits: &[TagEdit]) -> Vec<(String, String)> {
    edits
        .iter()
        .map(|e| (e.label.clone(), e.value.clone()))
        .collect()
}

#[test]
fn neutral_names_map_per_container() {
    let tags = vec![
        tag("bpm=127.98"),
        tag("INITIALKEY=8A"),
        tag("REPLAYGAIN_TRACK_GAIN=-3.20 dB"),
        tag("SOUNDCHECK=v1"),
    ];
    check_tags(&tags).expect("valid");
    let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    };
    assert_eq!(
        labels(&tag_edits(&tags, TagFamily::Id3)),
        pairs(&[
            ("TBPM", "128"),
            ("TXXX:BPM", "127.98"),
            ("TKEY", "8A"),
            ("TXXX:REPLAYGAIN_TRACK_GAIN", "-3.20 dB"),
            ("TXXX:SOUNDCHECK", "v1"),
        ])
    );
    assert_eq!(
        labels(&tag_edits(&tags, TagFamily::Vorbis)),
        pairs(&[
            ("BPM", "127.98"),
            ("INITIALKEY", "8A"),
            ("REPLAYGAIN_TRACK_GAIN", "-3.20 dB"),
            ("SOUNDCHECK", "v1"),
        ])
    );
}

#[test]
fn container_labels_and_bad_values_are_refused() {
    let refused = |text: &str, why: &str| {
        let tags = [tag(text)];
        match check_tags(&tags) {
            Err(Error::InvalidArgument(m)) => assert!(m.contains(why), "{text}: {m}"),
            other => panic!("{text}: {other:?}"),
        }
    };
    refused("TXXX:BPM=128", "use the neutral name (BPM for TXXX:BPM)");
    refused("TBPM=128", "looks like an ID3 frame id");
    refused("TKEY=8A", "looks like an ID3 frame id");
    refused("MY-TAG=x", "upper-case letters, digits or _");
    refused("BPM=fast", "positive number");
    refused("BPM=-1", "positive number");
    refused("COMMENT=a\0b", "NUL");
    let twice = [tag("BPM=128"), tag("bpm=129")];
    assert!(matches!(check_tags(&twice), Err(Error::InvalidArgument(m)) if m.contains("twice")));
    assert!(Tag::parse("BPM").is_err() && Tag::parse("=1").is_err());
    let eq = tag("COMMENT=a=b");
    assert_eq!((eq.name.as_str(), eq.value.as_str()), ("COMMENT", "a=b"));
}
