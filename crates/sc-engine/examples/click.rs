//! Plays a track with the click on its analysed grid on the default output device, printing the
//! position and the meters each second and the underrun count at the end. A listening check for
//! the player:
//!
//! ```text
//! cargo run -p sc-engine --release --features playback --example click -- <file> [seconds] [switches]
//! ```
//!
//! The grid comes from the analysis cache (analysing the file first when it is not cached), and
//! the music plays 6 dB under the click. With `switches` (for example 50 over a 600 s run), the
//! track's planned gain is -6 dB and the player switches between the original and the processed
//! version that many times, evenly spread, also lowering the monitor volume to -6 dB and back
//! every fifth switch: a soak of the gain ramps on the real device.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use sc_core::analysis::{AnalysisSettings, Model};
use sc_core::ipc::{Listen, Version};
use sc_core::{Bpm, DbFs};
use sc_engine::player::Player;
use sc_engine::{Analyzer, CancelToken, Track, TrackProgress};
use sc_io::cache::Cache;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = std::path::PathBuf::from(args.next().ok_or("usage: click <file> [seconds]")?);
    let seconds: u64 = args.next().map_or(Ok(20), |s| s.parse())?;
    let switches: u64 = args.next().map_or(Ok(0), |s| s.parse())?;

    let settings = AnalysisSettings {
        bpm_range: (Bpm(70.0), Bpm(180.0)),
        grid: true,
        model: Model::Small,
    };
    let cache = Cache::open(Cache::default_dir()?);
    let record = Analyzer::load(settings, Some(cache), CancelToken::new())?
        .analyze(&path)?
        .record;
    let grid = record.grid.ok_or("no grid for this file")?;
    println!(
        "{} BPM, {}, bar 1 at {:.3} s",
        grid.bpm.0,
        grid.meter,
        grid.anchor.to_seconds(record.spec.sample_rate).0
    );

    let (tx, rx) = mpsc::channel();
    let track = Arc::new(Track::open(&path, record.loudness.sample_peak, move |p| {
        if !matches!(p, TrackProgress::Decoded(_)) {
            let _ = tx.send(());
        }
    })?);
    let player = Player::new()?;
    let planned = if switches > 0 { -6.0 } else { 0.0 };
    player.load(Arc::clone(&track), DbFs(planned));
    player.set_grid(Some(Arc::new(grid)));
    player.set_click(true);
    player.play(None);
    rx.recv()?;
    let mut switched = 0;
    for second in 1..=seconds {
        std::thread::sleep(Duration::from_secs(1));
        while switched < switches && second * (switches + 1) >= (switched + 1) * seconds {
            switched += 1;
            let version = if switched % 2 == 1 {
                Version::Original
            } else {
                Version::Processed
            };
            player.set_listen(Listen {
                version,
                matched: false,
            });
            if switched % 5 == 0 {
                let down = switched % 10 == 5;
                player.set_volume(Some(DbFs(if down { -6.0 } else { 0.0 })));
            }
        }
        let status = player.status();
        if let Some(error) = status.error {
            return Err(error.into());
        }
        if let Some(state) = status.state {
            let level = |v: Option<f64>| v.map_or_else(|| "    -".to_owned(), |v| format!("{v:5.1}"));
            let meter = status.meter;
            println!(
                "{:>8.3} s  {}  {:?}  IN {} dBTP {} LUFS  OUT {} dBTP {} LUFS",
                state.position.to_seconds(track.sample_rate()).0,
                if state.playing { "playing" } else { "stopped" },
                status.listen.version,
                level(meter.and_then(|m| m.in_peak).map(|p| p.0)),
                level(meter.and_then(|m| m.in_momentary).map(|l| l.0)),
                level(meter.and_then(|m| m.out_peak).map(|p| p.0)),
                level(meter.and_then(|m| m.out_momentary).map(|l| l.0)),
            );
        }
    }
    let underruns = player.status().state.map_or(0, |s| s.underruns);
    println!("switches: {switched}, underruns: {underruns}");
    Ok(())
}
