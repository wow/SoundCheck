//! Plays a track with the click on its analysed grid on the default output device, printing the
//! position each second and the underrun count at the end. A listening check for the player:
//!
//! ```text
//! cargo run -p sc-engine --release --features playback --example click -- <file> [seconds]
//! ```
//!
//! The grid comes from the analysis cache (analysing the file first when it is not cached), and
//! the track plays at 0 dB with the music 6 dB under the click.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use sc_core::analysis::{AnalysisSettings, Model};
use sc_core::{Bpm, DbFs};
use sc_engine::player::Player;
use sc_engine::{Analyzer, CancelToken, Track, TrackProgress};
use sc_io::cache::Cache;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = std::path::PathBuf::from(args.next().ok_or("usage: click <file> [seconds]")?);
    let seconds: u64 = args.next().map_or(Ok(20), |s| s.parse())?;

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
    player.load(Arc::clone(&track), DbFs(0.0));
    player.set_grid(Some(Arc::new(grid)));
    player.set_click(true);
    player.play(None);
    rx.recv()?;
    for _ in 0..seconds {
        std::thread::sleep(Duration::from_secs(1));
        let status = player.status();
        if let Some(error) = status.error {
            return Err(error.into());
        }
        if let Some(state) = status.state {
            println!(
                "{:>8.3} s  {}",
                state.position.to_seconds(track.sample_rate()).0,
                if state.playing { "playing" } else { "stopped" }
            );
        }
    }
    let underruns = player.status().state.map_or(0, |s| s.underruns);
    println!("underruns: {underruns}");
    Ok(())
}
