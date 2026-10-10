//! Unit tests of `crates/sc-engine/src/txn/gate.rs`: closed while recovery runs, opened by any
//! result, and a cancel ends the wait.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;

fn finished() -> RecoveryStatus {
    RecoveryStatus::Finished {
        recovered: Vec::new(),
        pending: Vec::new(),
    }
}

#[test]
fn an_open_gate_does_not_wait() {
    for status in [
        finished(),
        RecoveryStatus::Failed {
            message: "journal".into(),
        },
        RecoveryStatus::Skipped {
            reason: "no backups".into(),
        },
    ] {
        let gate = RecoveryGate::new(status.clone());
        assert!(!gate.is_running());
        gate.wait(&CancelToken::new()).expect("open");
        assert_eq!(gate.status(), status);
    }
}

#[test]
fn a_waiting_job_goes_on_once_recovery_ends() {
    let gate = Arc::new(RecoveryGate::running());
    assert!(gate.is_running());
    let setter = {
        let gate = Arc::clone(&gate);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            gate.set(finished());
        })
    };
    let started = Instant::now();
    gate.wait(&CancelToken::new()).expect("opened");
    assert!(started.elapsed() >= Duration::from_millis(40));
    assert!(!gate.is_running());
    setter.join().expect("setter");
}

#[test]
fn a_cancel_ends_the_wait() {
    let gate = RecoveryGate::running();
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(gate.wait(&cancel), Err(Error::Cancelled)));
    assert!(gate.is_running());
}
