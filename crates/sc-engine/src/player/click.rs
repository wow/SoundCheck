//! The click: short sine bursts on the grid lines, pitched and levelled by where the line falls
//! in the bar. Bar lines are 1.5 kHz at -6 dBFS, the starts of the other beat groups (the "2"
//! of 2+2+2+3) 1.25 kHz at -10 dBFS, every other pulse 1 kHz at -14 dBFS. Each burst lasts
//! 5 ms: a 0.25 ms raised-cosine rise, so the click starts on the line's sample, then a
//! raised-cosine fall.

use sc_core::analysis::Meter;

/// Length of one click.
const CLICK_S: f64 = 0.005;
/// Rise time of one click.
const RISE_S: f64 = 0.000_25;

/// How strongly a grid line is marked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    /// Beat 1 of a bar.
    Bar,
    /// The first pulse of a beat group other than the first.
    Group,
    /// Any other pulse.
    Pulse,
}

impl Accent {
    /// The accent of pulse `pulse` of a bar of `meter` (pulse 0 is beat 1).
    #[must_use]
    pub fn of(meter: &Meter, pulse: usize) -> Self {
        if pulse == 0 {
            return Self::Bar;
        }
        let mut start = 0;
        for &group in &meter.grouping {
            if pulse == start {
                return Self::Group;
            }
            start += usize::from(group);
        }
        Self::Pulse
    }

    fn sound(self) -> (f64, f64) {
        match self {
            Self::Bar => (1500.0, -6.0),
            Self::Group => (1250.0, -10.0),
            Self::Pulse => (1000.0, -14.0),
        }
    }
}

/// The three click sounds, mono, rendered once at one sample rate.
#[derive(Debug, Clone)]
pub struct Clicks {
    bar: Vec<f32>,
    group: Vec<f32>,
    pulse: Vec<f32>,
}

impl Clicks {
    /// The clicks at `sample_rate`.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        let render = |accent: Accent| {
            let (hz, db) = accent.sound();
            let rate = f64::from(sample_rate);
            let amplitude = 10f64.powf(db / 20.0);
            let len = (CLICK_S * rate).round().max(1.0);
            let rise = (RISE_S * rate).max(1.0);
            (0..to_count(len))
                .map(|n| {
                    let t = count_f64(n);
                    let envelope = if t < rise {
                        0.5 - 0.5 * (std::f64::consts::PI * t / rise).cos()
                    } else {
                        0.5 + 0.5 * (std::f64::consts::PI * (t - rise) / (len - rise)).cos()
                    };
                    // A cosine phase: the burst's first sample is its first non-zero value
                    // only through the envelope, never through the sine's own zero crossing.
                    let tone =
                        (std::f64::consts::TAU * hz * t / rate + 0.5 * std::f64::consts::PI).sin();
                    to_f32(amplitude * envelope * tone)
                })
                .collect()
        };
        Self {
            bar: render(Accent::Bar),
            group: render(Accent::Group),
            pulse: render(Accent::Pulse),
        }
    }

    /// The samples of one click.
    #[must_use]
    pub fn sound(&self, accent: Accent) -> &[f32] {
        match accent {
            Accent::Bar => &self.bar,
            Accent::Group => &self.group,
            Accent::Pulse => &self.pulse,
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a positive sample count
fn to_count(n: f64) -> usize {
    n as usize
}

#[allow(clippy::cast_precision_loss)] // a sample index within one click
fn count_f64(n: usize) -> f64 {
    n as f64
}

#[allow(clippy::cast_possible_truncation)] // a sample within +/-1
fn to_f32(x: f64) -> f32 {
    x as f32
}

#[cfg(test)]
mod tests {
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
}
