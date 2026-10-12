//! Download a registry model into the app-data dir, as the app would. Used to
//! set up the ignored real-model tests:
//! `cargo run --example fetch_models [model-id] [app-data-dir]`
//! (model id defaults to the speech model, e.g. `qwen3.5-2b` for Smart).

use purl_lib::engine::{download::download_model, models::find, stt::MODEL_ID};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn main() {
    let mut args = std::env::args().skip(1);
    let id = args.next().unwrap_or_else(|| MODEL_ID.into());
    let app_data = args.next().map(PathBuf::from).unwrap_or_else(|| {
        let home = PathBuf::from(std::env::var("HOME").expect("HOME"));
        home.join("Library/Application Support/app.purl.desktop")
    });
    let spec = find(&id).unwrap_or_else(|| panic!("no model {id} in the registry"));
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
