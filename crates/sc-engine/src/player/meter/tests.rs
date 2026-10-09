//! Unit tests of the private parts of `crates/sc-engine/src/player/meter.rs`: the history, the
//! `None` cases, reset and priming, and OUT as IN plus the planned gain.
use sc_core::{AudioSpec, testsig};

use super::*;

fn reading(start: u64, end: u64) -> Reading {
    Reading {
        start,
        end,
        peak: 0.5,
        momentary: Some(-20.0),
    }
}

#[test]
fn the_reading_for_a_position_is_the_block_that_holds_it() {
    let meters = Meters::new();
    for b in 0..4 {
        meters.push(reading(1_000 + b * 1_024, 1_000 + (b + 1) * 1_024));
    }
    assert_eq!(meters.at(SampleIndex(999)), None, "before the first block");
    let at = |f| meters.at(SampleIndex(f)).map(|m| m.position.0);
    assert_eq!(at(1_000), Some(2_024));
    assert_eq!(at(2_023), Some(2_024));
    assert_eq!(at(2_024), Some(3_048));
    assert_eq!(at(5_096), Some(5_096), "the end of the last block");
    assert_eq!(at(5_096 + 1_024), Some(5_096), "up to a block past it");
    assert_eq!(at(5_096 + 1_025), None);
}

#[test]
fn the_history_keeps_the_newest_blocks_without_growing() {
    let meters = Meters::new();
    let blocks = HISTORY_BLOCKS as u64 + 10;
    for b in 0..blocks {
        meters.push(reading(b * 1_024, (b + 1) * 1_024));
    }
    let history = meters.lock();
    assert_eq!(history.readings.len(), HISTORY_BLOCKS);
    assert_eq!(history.readings.capacity(), HISTORY_BLOCKS);
    assert_eq!(history.readings[0].start, 10 * 1_024);
    drop(history);
    assert_eq!(meters.at(SampleIndex(5 * 1_024)), None, "dropped");
}

#[test]
fn out_is_in_plus_the_planned_gain_rounded_to_a_thousandth() {
    let meters = Meters::new();
    meters.push(Reading {
        start: 0,
        end: 1_024,
        peak: 0.5,
        momentary: Some(-12.345_67),
    });
    meters.set_out_offset(DbFs(-2.345_6));
    let f = meters.latest().expect("a reading");
    assert_eq!(f.in_peak, Some(DbTp(-6.021)));
    assert_eq!(f.in_momentary, Some(Lufs(-12.346)));
    assert_eq!(f.out_peak, Some(DbTp(-8.366)));
    assert_eq!(f.out_momentary, Some(Lufs(-14.691)));
}

#[test]
fn silence_reads_none_and_momentary_waits_for_400_ms() {
    let meters = Arc::new(Meters::new());
    let mut metering = Metering::new(44_100, 2, Arc::clone(&meters)).expect("a meter");
    let silence = vec![0.0_f32; 2 * 1_024];
    metering.push(&silence, SampleIndex(1_024));
    let f = meters.latest().expect("a reading");
    assert_eq!((f.in_peak, f.in_momentary), (None, None));

    let tone = testsig::sine(AudioSpec::CD, 1_000.0, 0.1, 1.0);
    let mut end = 1_024;
    let mut first_momentary = None;
    for block in tone.data.chunks(2 * 1_024) {
        end += 1_024;
        metering.push(block, SampleIndex(end));
        if first_momentary.is_none() && meters.latest().and_then(|f| f.in_momentary).is_some() {
            first_momentary = Some(end);
        }
    }
    // 400 ms is 17,640 frames: the 18th block after the start is the first with a reading.
    assert_eq!(first_momentary, Some(18 * 1_024));
    assert!(meters.latest().and_then(|f| f.in_peak).is_some());
}

#[test]
fn a_reset_empties_the_history_and_restarts_the_window() {
    let meters = Arc::new(Meters::new());
    let mut metering = Metering::new(48_000, 2, Arc::clone(&meters)).expect("a meter");
    let tone = testsig::sine(AudioSpec::new(48_000, 2), 1_000.0, 0.1, 1.0);
    for (b, block) in tone.data.chunks(2 * 1_024).enumerate() {
        metering.push(block, SampleIndex((b as u64 + 1) * 1_024));
    }
    assert!(meters.latest().and_then(|f| f.in_momentary).is_some());
    metering.reset();
    assert_eq!(meters.latest(), None);
    metering.push(&tone.data[..2 * 1_024], SampleIndex(90_000));
    let f = meters.latest().expect("a reading");
    assert_eq!(f.in_momentary, None, "a fresh window");
    assert_eq!(f.position, SampleIndex(90_000));
}

#[test]
fn priming_measures_nothing_and_keeps_the_window_empty() {
    let meters = Arc::new(Meters::new());
    let mut metering = Metering::new(44_100, 2, Arc::clone(&meters)).expect("a meter");
    assert_eq!(metering.prime_frames(), 4_410, "100 ms");
    let tone = testsig::sine(AudioSpec::CD, 1_000.0, 0.1, 2.0);
    let (before, after) = tone.data.split_at(2 * 44_100);
    metering.reset();
    metering.prime(before);
    assert_eq!(meters.latest(), None, "no reading");
    let mut end = 44_100;
    let mut first_momentary = None;
    for block in after.chunks(2 * 1_024) {
        end += 1_024;
        metering.push(block, SampleIndex(end));
        if first_momentary.is_none() && meters.latest().and_then(|f| f.in_momentary).is_some() {
            first_momentary = Some(end - 44_100);
        }
    }
    // As without priming: the 18th block measured is the first with 400 ms (17,640 frames).
    assert_eq!(first_momentary, Some(18 * 1_024));
    // Frames that are not whole, or none at all, prime with what is there.
    metering.reset();
    metering.prime(&before[..7]);
    metering.prime(&[]);
    assert_eq!(meters.latest(), None);
}

#[test]
fn mono_reads_as_dual_mono_like_the_analysis() {
    let stereo = testsig::sine(AudioSpec::CD, 1_000.0, 0.1, 1.0);
    let mono: Vec<f32> = stereo.data.iter().step_by(2).copied().collect();
    let read = |data: &[f32], channels: u16| {
        let meters = Arc::new(Meters::new());
        let mut metering = Metering::new(44_100, channels, Arc::clone(&meters)).expect("a meter");
        metering.push(data, SampleIndex(44_100));
        meters.latest().expect("a reading")
    };
    assert_eq!(read(&mono, 1), read(&stereo.data, 2));
}

#[test]
fn any_channel_count_but_none_is_metered() {
    let meters = Arc::new(Meters::new());
    let mut metering = Metering::new(48_000, 3, Arc::clone(&meters)).expect("three channels");
    // Left and right silent, a 1 kHz sine at half scale in the centre: the centre counts.
    let tone = testsig::sine(AudioSpec::new(48_000, 1), 1_000.0, 0.5, 0.1);
    let block: Vec<f32> = tone.data.iter().flat_map(|&c| [0.0, 0.0, c]).collect();
    metering.push(&block, SampleIndex(4_800));
    let peak = meters.latest().and_then(|f| f.in_peak).expect("a peak");
    assert!((peak.0 - 20.0 * 0.5_f64.log10()).abs() < 0.01, "{peak:?}");
    assert!(matches!(
        Metering::new(44_100, 0, meters),
        Err(Error::InvalidArgument(_))
    ));
}

#[test]
fn a_reading_since_the_previous_one_holds_every_peak_in_between() {
    let meters = Meters::new();
    for b in 0..10_u64 {
        meters.push(Reading {
            start: b * 1_024,
            end: (b + 1) * 1_024,
            // One loud block, the fourth.
            peak: if b == 3 { 1.0 } else { 0.1 },
            momentary: Some(-20.0 - f64::from(u32::try_from(b).unwrap())),
        });
    }
    let peak = |f: Option<MeterFrame>| f.and_then(|f| f.in_peak).map(|p| p.0);
    // Read at blocks 1 and 5: the second reading covers blocks 2 to 5, the loud one included.
    let first = meters.since(SampleIndex(1_500), None);
    assert_eq!(peak(first), Some(-20.0));
    let second = meters.since(SampleIndex(5_500), first.map(|f| f.position));
    assert_eq!(peak(second), Some(0.0));
    assert_eq!(
        second.and_then(|f| f.in_momentary),
        Some(Lufs(-25.0)),
        "the heard block's"
    );
    // Without a previous reading, or after a seek back, every stored block up to the heard one
    // (the history holds only what was measured since the seek or load).
    assert_eq!(peak(meters.since(SampleIndex(5_500), None)), Some(0.0));
    assert_eq!(
        peak(meters.since(SampleIndex(5_500), Some(SampleIndex(9_000)))),
        Some(0.0)
    );
    assert_eq!(
        peak(meters.since(SampleIndex(1_500), Some(SampleIndex(9_000)))),
        Some(-20.0)
    );
    // `at` is the heard block alone.
    assert_eq!(peak(meters.at(SampleIndex(5_500))), Some(-20.0));
}

#[test]
fn frames_say_when_the_track_is_a_mono_fold() {
    let meters = Meters::new();
    meters.push(reading(0, 1_024));
    assert!(!meters.latest().expect("a reading").folded);
    meters.set_folded(true);
    assert!(meters.latest().expect("a reading").folded);
}
