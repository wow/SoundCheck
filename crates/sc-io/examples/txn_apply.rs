//! Changes one file in place through a write transaction; the crash matrix
//! (`tests/crash.rs`) runs it with `SC_TEST_CRASH_AFTER_STEP` set so it aborts after a step.
//! Built only with the `crash-test` feature, which must never be enabled in a shipped build.
//!
//! Usage: `txn_apply <file> <backup root> [gain dB, default -3]`. Prints the output's BLAKE3.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use sc_core::RenderRequest;
use sc_io::txn::{TxnOptions, apply_in_place, hex};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(file), Some(root)) = (args.first(), args.get(1)) else {
        eprintln!("usage: txn_apply <file> <backup root> [gain dB]");
        return ExitCode::from(2);
    };
    let gain_db = args.get(2).and_then(|g| g.parse().ok()).unwrap_or(-3.0);
    let req = RenderRequest {
        gain_db,
        ..RenderRequest::default()
    };
    let opts = TxnOptions::new(PathBuf::from(root));
    match apply_in_place(
        &PathBuf::from(file),
        &req,
        &opts,
        &AtomicBool::new(false),
    ) {
        Ok(report) => {
            println!("{}", hex(&report.output_blake3));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
