//! Download the Parakeet speech model into the app-data dir, as the app's
//! onboarding would. Used to set up the ignored real-model test:
//! `cargo run --example fetch_models [app-data-dir]`.

use purl_lib::engine::{download::download_model, models::find, stt::MODEL_ID};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn main() {
    let app_data = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = PathBuf::from(std::env::var("HOME").expect("HOME"));
            home.join("Library/Application Support/app.purl.desktop")
        });
    let spec = find(MODEL_ID).expect("registry has the speech model");
    let t0 = std::time::Instant::now();
    let result = download_model(spec, &app_data, &AtomicBool::new(false), &mut |p| {
        eprint!("\r{:>4} / {} MB", p.downloaded >> 20, p.total >> 20);
    });
    eprintln!(
        "\n{result:?} in {:?} -> {}",
        t0.elapsed(),
        app_data.display()
    );
}
