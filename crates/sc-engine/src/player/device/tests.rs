//! Unit tests of the private parts of `crates/sc-engine/src/player/device.rs`.
use super::*;

#[test]
fn only_a_lost_or_rerouted_device_stops_playback() {
    use cpal::ErrorKind::{
        BackendError, DeviceBusy, DeviceChanged, DeviceNotAvailable, RealtimeDenied,
        StreamInvalidated, Xrun,
    };
    for kind in [
        DeviceNotAvailable,
        DeviceChanged,
        StreamInvalidated,
        BackendError,
    ] {
        assert!(stops_playback(kind), "{kind:?}");
    }
    for kind in [Xrun, DeviceBusy, RealtimeDenied] {
        assert!(!stops_playback(kind), "{kind:?}");
    }
}
