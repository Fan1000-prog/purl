// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().skip(1).any(|a| a == "--smoke") {
        std::process::exit(purl_lib::smoke::run());
    }
    purl_lib::run()
}
