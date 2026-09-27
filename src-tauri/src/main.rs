//! Desktop entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

fn main() {
    soundcheck_desktop_lib::run();
}
