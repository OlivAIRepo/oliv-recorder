#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use log;

fn main() {
    // The logger is initialized by tauri-plugin-log (file + stdout) inside run().
    log::info!("Starting application...");
    app_lib::run();
}
