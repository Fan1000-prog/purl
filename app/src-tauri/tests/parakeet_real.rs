//! Real-model check for the speech engine. Ignored by default: it needs the
//! ~670 MB Parakeet model in the app-data dir. Fetch it with
//! `cargo run --example fetch_models`, then
//! `cargo test --test parakeet_real -- --ignored`.

use purl_lib::engine::models::model_dir;
use purl_lib::engine::stt::{ParakeetEngine, SttEngine, MODEL_ID};
use std::path::PathBuf;
use std::time::Instant;

/// Tauri's `app_data_dir()` for `app.purl.desktop`, outside a running app.
fn app_data() -> PathBuf {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var("APPDATA").expect("APPDATA"))
    } else {
        PathBuf::from(std::env::var("HOME").expect("HOME")).join("Library/Application Support")
    };
    base.join("app.purl.desktop")
}

#[test]
#[ignore = "needs the 670 MB Parakeet model; run: cargo test -- --ignored parakeet_real"]
fn parakeet_real_hello() {
    let engine = ParakeetEngine::new(model_dir(&app_data(), MODEL_ID));
    let pcm = transcribe_rs::audio::read_wav_samples(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello-purl.wav"),
    )
    .unwrap();

    let t0 = Instant::now();
    engine.warm();
    let text = engine.transcribe(&pcm).unwrap();
    let first = t0.elapsed();
    let t1 = Instant::now();
    engine.transcribe(&pcm).unwrap();
    eprintln!(
        "transcript: {text:?}\nload + first transcribe: {first:?}\nwarm transcribe: {:?}",
        t1.elapsed()
    );

    // Parakeet hears the made-up word "Purl" as "Pearl", so the check sticks
    // to ordinary words; spelling "Purl" right is the dictionary's job.
    let lower = text.to_lowercase();
    assert!(
        lower.contains("hello") && lower.contains("dictation test"),
        "{text}"
    );
}

/// The decoder must honour Parakeet's TDT duration head: without it, it
/// emitted one token up to 10 times on a frame ("Francis X X X … Xavier").
#[test]
#[ignore = "needs the 670 MB Parakeet model; run: cargo test -- --ignored parakeet_real"]
fn parakeet_real_no_repeated_tokens() {
    let engine = ParakeetEngine::new(model_dir(&app_data(), MODEL_ID));
    let pcm = transcribe_rs::audio::read_wav_samples(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/librispeech-1089-134686-0033.wav"),
    )
    .unwrap();
    engine.warm();
    let text = engine.transcribe(&pcm).unwrap();
    eprintln!("transcript: {text:?}");

    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect();
    assert!(
        !words.windows(3).any(|w| w[0] == w[1] && w[1] == w[2]),
        "a word repeats 3 times: {text}"
    );
    assert!(words.contains(&"xavier".to_string()), "{text}");
}
