//! HaryX for Windows. No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    haryx_app::run();
}
