//! macOS stress check of the resource fork in an in-place change: the fork is written through
//! `<file>/..namedfork/rsrc` and the change applied at once, while other threads start
//! processes and keep every core busy. A process being started holds a duplicate of every open
//! descriptor for an instant, so the test's `close` of the fork is sometimes not the last one,
//! and APFS lists the fork as an extended attribute only after the last one closes; the output
//! must carry the fork anyway (and report nothing missing).
//!
//! Opt-in, as it takes about a minute and a half:
//! `SC_FORK_STRESS=1 cargo test -p sc-io --test fork_stress -- --ignored --nocapture`
//! (`SC_FORK_STRESS_ITERATIONS`, default 500).
#![cfg(target_os = "macos")]

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::{Library, wav};
use sc_core::RenderRequest;
use sc_io::txn::{self, TxnOptions};

static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Threads starting processes, and threads only spinning.
const SPAWNING_THREADS: usize = 8;

fn listed(path: &Path) -> bool {
    xattr::list(path)
        .expect("extended attributes")
        .any(|n| n.as_os_str() == "com.apple.ResourceFork")
}

#[test]
#[ignore = "stress run, about 90 s: SC_FORK_STRESS=1"]
fn a_resource_fork_written_just_before_a_change_always_reaches_the_output() {
    if std::env::var_os("SC_FORK_STRESS").is_none() {
        eprintln!("skipped: set SC_FORK_STRESS=1");
        return;
    }
    let iterations: usize = std::env::var("SC_FORK_STRESS_ITERATIONS").map_or(500, |v| {
        v.parse().expect("SC_FORK_STRESS_ITERATIONS is a number")
    });
    let stop = Arc::new(AtomicBool::new(false));
    let spinners = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let load: Vec<_> = (0..SPAWNING_THREADS + spinners)
        .map(|k| {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if k < SPAWNING_THREADS {
                        let _ = std::process::Command::new("/usr/bin/true").status();
                    } else {
                        std::hint::spin_loop();
                    }
                }
            })
        })
        .collect();
    let fork: Vec<u8> = (0..70_000_u32).map(|i| (i % 253) as u8).collect();
    let req = RenderRequest {
        gain_db: -2.0,
        ..RenderRequest::default()
    };
    let (mut pending, mut lost) = (0, Vec::new());
    for i in 0..iterations {
        let lib = Library::new();
        let path = lib.add("fork.wav", &wav(3000, 31));
        std::fs::write(path.join("..namedfork/rsrc"), &fork).expect("resource fork written");
        if !listed(&path) {
            pending += 1;
        }
        let report =
            txn::apply_in_place(&path, &req, &TxnOptions::new(&lib.backups), &NOT_CANCELLED)
                .expect("applied");
        let out = std::fs::read(path.join("..namedfork/rsrc"));
        if !report.notes.is_empty() || !matches!(&out, Ok(v) if *v == fork) {
            lost.push(format!(
                "iteration {i}: output fork {:?}, notes {:?}",
                out.map(|v| v.len()),
                report.notes
            ));
        }
    }
    stop.store(true, Ordering::Relaxed);
    for t in load {
        t.join().expect("load thread");
    }
    eprintln!(
        "{iterations} changes: fork not yet listed right before {pending}, output without it {}",
        lost.len()
    );
    assert!(lost.is_empty(), "{lost:#?}");
}
