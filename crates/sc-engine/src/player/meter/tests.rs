//! Unit tests of the private parts of `crates/sc-engine/src/player/meter.rs`: the history, the
//! `None` cases, reset, and OUT as IN plus the planned gain.
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
fn more_than_two_channels_are_refused() {
    let meters = Arc::new(Meters::new());
    assert!(matches!(
        Metering::new(44_100, 3, Arc::clone(&meters)),
        Err(Error::InvalidArgument(_))
    ));
    assert!(Metering::new(44_100, 0, meters).is_err());
}
