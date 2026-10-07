//! Runs one write transaction; the crash matrix (`tests/crash.rs`) runs it with
//! `SC_TEST_CRASH_AFTER_STEP` set so it aborts at a crash point. Built only with the
//! `crash-test` feature, which must never be enabled in a shipped build.
//!
//! Usage:
//! - `txn_apply apply <file> <backup root>`: change the file in place by -3 dB;
//! - `txn_apply folder <file> <backup root> <folder>`: write it changed into the folder;
//! - `txn_apply undo <file> <backup root>`: undo its newest change.
//!
//! Prints the BLAKE3 of the file written.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use sc_core::RenderRequest;
use sc_io::txn::{TxnOptions, apply_in_place, apply_to_folder, hex, undo};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(PathBuf::from);
    let (Some(mode), Some(file), Some(root)) = (args.first(), arg(1), arg(2)) else {
        eprintln!("usage: txn_apply apply|folder|undo <file> <backup root> [folder]");
        return ExitCode::from(2);
    };
    let req = RenderRequest {
        gain_db: -3.0,
        ..RenderRequest::default()
    };
    let opts = TxnOptions::new(root.clone());
    let cancel = AtomicBool::new(false);
    let hash = match (mode.as_str(), arg(3)) {
        ("apply", _) => apply_in_place(&file, &req, &opts, &cancel).map(|r| r.output_blake3),
        ("folder", Some(out)) => {
            apply_to_folder(&file, &out, &req, &opts, &cancel).map(|r| r.output_blake3)
        }
        ("undo", _) => undo(&file, &root).map(|r| r.restored_blake3),
        _ => {
            eprintln!("unknown mode {mode}");
            return ExitCode::from(2);
        }
    };
    match hash {
        Ok(h) => {
            println!("{}", hex(&h));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
