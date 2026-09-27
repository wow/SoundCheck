//! Unit tests of the private parts of `crates/sc-engine/src/player/click.rs`.
use super::*;

#[test]
fn accents_follow_the_grouping() {
    use Accent::{Bar, Group, Pulse};
    let aksak = Meter::nine_eight_aksak();
    let accents: Vec<Accent> = (0..9).map(|p| Accent::of(&aksak, p)).collect();
    assert_eq!(
        accents,
        [Bar, Pulse, Group, Pulse, Group, Pulse, Group, Pulse, Pulse]
    );
    let four = Meter::four_four();
    assert_eq!(Accent::of(&four, 0), Bar);
    assert_eq!(
        Accent::of(&four, 1),
        Group,
        "every quarter starts a group of one"
    );
}

#[test]
fn clicks_start_at_their_first_sample_and_stay_under_their_level() {
    let clicks = Clicks::new(48_000);
    for (accent, db) in [
        (Accent::Bar, -6.0),
        (Accent::Group, -10.0),
        (Accent::Pulse, -14.0),
    ] {
        let s = clicks.sound(accent);
        assert_eq!(s.len(), 240, "5 ms at 48 kHz");
        assert!(
            s[0] == 0.0 && s[1].abs() > 0.0,
            "{accent:?}: starts on its first sample"
        );
        let peak = s.iter().fold(0.0_f32, |m, x| m.max(x.abs()));
        let limit = 10f32.powf(db / 20.0);
        assert!(
            peak <= limit && peak > 0.8 * limit,
            "{accent:?}: {peak} vs {limit}"
        );
    }
}
