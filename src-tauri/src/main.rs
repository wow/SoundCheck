//! Desktop entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    soundcheck_desktop_lib::run();
}
