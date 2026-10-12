//! Reproducible accuracy and speed benchmark (see bench/README.md).
//!
//! `cargo run --release --example bench -- [--cache DIR] [--model-dir DIR]
//!  [--whisper-cli PATH] [--out FILE]`
//!
//! Parakeet v3 runs through the app's own engine; whisper.cpp base.en, the
//! previous engine, runs through `whisper-cli` and exists only here.

mod wer;

use purl_lib::cleanup::smart_clean;
use purl_lib::dictionary::{Dictionary, SEED};
use purl_lib::engine::llm::{self, LlamaServerEngine, LlmEngine};
use purl_lib::engine::models::model_dir;
use purl_lib::engine::sidecar::llama_server_path;
use purl_lib::engine::stt::{ParakeetEngine, SttEngine, MODEL_ID};
use purl_lib::engine::EngineState;
use purl_lib::fastclean;
use purl_lib::StructureMode;
use serde_json::{json, Value};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SAMPLE_RATE: f64 = 16_000.0;
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
const WHISPER_TIMEOUT: Duration = Duration::from_secs(120);
/// Fast cleanup takes microseconds; repeat each call to get above timer noise.
const CLEANUP_REPS: u32 = 1_000;
/// The Smart model the app recommends, and the app's budget for it: a call
/// that runs longer falls back to Fast.
const SMART_MODEL: &str = "qwen3.5-2b";
const SMART_BUDGET: Duration = Duration::from_secs(6);
/// Smart calls per transcript, after one untimed warm-up call.
const SMART_REPS: usize = 3;

struct Clip {
    file: String,
    path: PathBuf,
    reference: String,
    pcm: Vec<f32>,
}

impl Clip {
    fn secs(&self) -> f64 {
        self.pcm.len() as f64 / SAMPLE_RATE
    }
}

struct ClipResult {
    hypothesis: String,
    processing: Duration,
}

struct Args {
    cache: PathBuf,
    app_data: PathBuf,
    model_dir: PathBuf,
    whisper_cli: PathBuf,
    out: Option<PathBuf>,
}

fn main() {
    let args = parse_args();
    let repo = repo_root();
    let sets = [
        ("librispeech_100", args.cache.join("librispeech-100")),
        ("dictation_10", repo.join("bench/dictation")),
    ];
    let data: Vec<(&str, Vec<Clip>)> = sets
        .iter()
        .map(|(name, dir)| (*name, load_set(dir)))
        .collect();
    for (name, clips) in &data {
        eprintln!("{name}: {} clips", clips.len());
    }
    let ggml = args.cache.join("ggml-base.en.bin");
    let scratch = args.cache.join("runs");
    std::fs::create_dir_all(&scratch).expect("create scratch dir");

    let machine = machine(&repo);
    let (parakeet, dictation_transcripts) = bench_parakeet(&args.model_dir, &scratch, &data);
    let whisper_gpu = bench_whisper(&args.whisper_cli, &ggml, &scratch, &data, true);
    let whisper_cpu = bench_whisper(&args.whisper_cli, &ggml, &scratch, &data, false);
    let smart = bench_smart_cleanup(&args.app_data, &dictation_transcripts);

    let mut machine = machine;
    machine["load_avg_1m_at_end"] = json!(load_avg());
    let report = json!({
        "date": chrono::Local::now().format("%Y-%m-%d").to_string(),
        "machine": machine,
        "datasets": {
            "librispeech_100": {
                "source": "LibriSpeech test-clean, first 100 utterances by sorted id (openslr.org/resources/12)",
                "licence": "CC-BY-4.0",
                "clips": data[0].1.len(),
                "audio_secs": round(total_secs(&data[0].1), 1),
            },
            "dictation_10": {
                "source": "bench/dictation/manifest.tsv: dictation-style text read by macOS `say`",
                "synthetic": true,
                "clips": data[1].1.len(),
                "audio_secs": round(total_secs(&data[1].1), 1),
            },
        },
        "method": {
            "wer": "corpus WER (total word edits / total reference words) after lowercasing and stripping punctuation; numbers are not normalized",
            "rtf": "processing time / audio duration per clip, lower is better; model already loaded",
            "cold_load": "fresh process, model files possibly in the OS file cache from earlier runs",
        },
        "stt": [parakeet, whisper_gpu, whisper_cpu],
        "cleanup": [
            bench_fast_cleanup(&dictation_transcripts),
            smart,
        ],
        "windows": "not measured yet",
    });

    let text = serde_json::to_string_pretty(&report).expect("serialize report");
    println!("{text}");
    let out = args.out.unwrap_or_else(|| {
        let name = format!(
            "{}-{}.json",
            report["date"].as_str().unwrap(),
            slug(report["machine"]["cpu"].as_str().unwrap_or("unknown"))
        );
        repo.join("bench/results").join(name)
    });
    std::fs::write(&out, text + "\n").expect("write results");
    eprintln!("wrote {}", out.display());
}

fn parse_args() -> Args {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let app_data = home.join("Library/Application Support/app.purl.desktop");
    let mut args = Args {
        cache: std::env::var("PURL_BENCH_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join("Library/Caches/purl-bench")),
        model_dir: model_dir(&app_data, MODEL_ID),
        app_data,
        whisper_cli: PathBuf::from("whisper-cli"),
        out: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--cache" => args.cache = value.into(),
            "--model-dir" => args.model_dir = value.into(),
            "--whisper-cli" => args.whisper_cli = value.into(),
            "--out" => args.out = Some(value.into()),
            _ => panic!("unknown flag {flag}"),
        }
    }
    args
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A set's clips from its manifest.tsv (`file  source  voice  text`).
fn load_set(dir: &Path) -> Vec<Clip> {
    let manifest = dir.join("manifest.tsv");
    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("{}: {e}. Run bench/fetch.sh first.", manifest.display()));
    text.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            let (file, reference) = (cols[0], cols[cols.len() - 1]);
            let path = dir.join(file);
            let pcm = transcribe_rs::audio::read_wav_samples(&path)
                .unwrap_or_else(|e| panic!("{file}: {e}"));
            Clip {
                file: file.to_string(),
                path,
                reference: reference.to_string(),
                pcm,
            }
        })
        .collect()
}

/// Parakeet results, plus its dictation-set transcripts for the cleanup run.
fn bench_parakeet(dir: &Path, scratch: &Path, data: &[(&str, Vec<Clip>)]) -> (Value, Vec<String>) {
    eprintln!("parakeet: loading from {}", dir.display());
    let engine = ParakeetEngine::new(dir.to_path_buf());
    let t0 = Instant::now();
    engine.warm();
    loop {
        match engine.status() {
            EngineState::Ready => break,
            EngineState::Loading if t0.elapsed() < LOAD_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(5))
            }
            other => panic!("parakeet did not load: {other:?}"),
        }
    }
    let load = t0.elapsed();

    let mut out = json!({
        "engine": "parakeet-tdt-0.6b-v3",
        "config": "int8 ONNX, in-process via transcribe-rs (the app's engine), CPU",
        "cold_load_ms": ms(load),
    });
    let mut dictation = Vec::new();
    for (name, clips) in data {
        let results: Vec<ClipResult> = clips
            .iter()
            .map(|clip| {
                let t = Instant::now();
                let hypothesis = engine
                    .transcribe(&clip.pcm)
                    .unwrap_or_else(|e| panic!("{}: {e}", clip.file));
                ClipResult {
                    hypothesis,
                    processing: t.elapsed(),
                }
            })
            .collect();
        out[name] = score(clips, &results);
        save_transcripts(scratch, "parakeet", name, clips, &results);
        if *name == "dictation_10" {
            dictation = results.into_iter().map(|r| r.hypothesis).collect();
        }
    }
    (out, dictation)
}

fn bench_whisper(
    cli: &Path,
    ggml: &Path,
    scratch: &Path,
    data: &[(&str, Vec<Clip>)],
    gpu: bool,
) -> Value {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let (label, flags): (&str, Vec<String>) = if gpu {
        ("Metal GPU (whisper-cli default)", vec![])
    } else {
        ("CPU only (-ng)", vec!["-ng".into()])
    };
    eprintln!("whisper base.en: {label}");
    let mut cold_load = None;
    let mut out = json!({
        "engine": "whisper.cpp base.en",
        "config": format!("{label}, {threads} threads, {}", whisper_version(cli)),
    });
    for (name, clips) in data {
        let results: Vec<ClipResult> = clips
            .iter()
            .map(|clip| {
                let (hypothesis, load, processing) =
                    run_whisper(cli, ggml, &clip.path, scratch, threads, &flags);
                cold_load.get_or_insert(load);
                ClipResult {
                    hypothesis,
                    processing,
                }
            })
            .collect();
        out[name] = score(clips, &results);
        let engine = if gpu { "whisper-gpu" } else { "whisper-cpu" };
        save_transcripts(scratch, engine, name, clips, &results);
    }
    out["cold_load_ms"] = json!(cold_load.map(ms));
    out
}

/// One whisper-cli run: transcript, its own load time, and processing time
/// (its total minus load). Output goes to files, not pipes, so a stuck run
/// can be timed out and killed.
fn run_whisper(
    cli: &Path,
    ggml: &Path,
    wav: &Path,
    scratch: &Path,
    threads: usize,
    flags: &[String],
) -> (String, Duration, Duration) {
    let base = scratch.join("whisper");
    let log = scratch.join("whisper.log");
    let mut child = Command::new(cli)
        .arg("-m")
        .arg(ggml)
        .arg("-f")
        .arg(wav)
        .args(["-t", &threads.to_string(), "-nt", "-otxt", "-of"])
        .arg(&base)
        .args(flags)
        .stdout(Stdio::null())
        .stderr(File::create(&log).expect("create whisper log"))
        .spawn()
        .unwrap_or_else(|e| panic!("{}: {e}. Install whisper.cpp.", cli.display()));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for whisper-cli") {
            break status;
        }
        if started.elapsed() > WHISPER_TIMEOUT {
            let _ = child.kill();
            panic!("whisper-cli timed out on {}", wav.display());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let log = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(status.success(), "whisper-cli failed:\n{log}");
    let load = timing(&log, "load time").expect("whisper load time");
    let total = timing(&log, "total time").expect("whisper total time");
    let text = std::fs::read_to_string(base.with_extension("txt")).expect("whisper transcript");
    (text.trim().to_string(), load, total.saturating_sub(load))
}

/// `whisper_print_timings:     load time =    68.31 ms` → 68.31 ms.
fn timing(log: &str, label: &str) -> Option<Duration> {
    let line = log
        .lines()
        .find(|l| l.starts_with("whisper_print_timings") && l.contains(label))?;
    let value = line.split('=').nth(1)?.split("ms").next()?.trim();
    value
        .parse::<f64>()
        .ok()
        .map(|v| Duration::from_secs_f64(v / 1000.0))
}

fn whisper_version(cli: &Path) -> String {
    Command::new(cli)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            let all = [o.stdout, o.stderr].concat();
            String::from_utf8_lossy(&all)
                .lines()
                .find(|l| l.contains("version"))
                .map(|l| l.trim().to_string())
        })
        .unwrap_or_else(|| "whisper.cpp version unknown".into())
}

/// Per-clip reference and transcript, kept in the cache for error analysis.
fn save_transcripts(dir: &Path, engine: &str, set: &str, clips: &[Clip], results: &[ClipResult]) {
    let rows: String = clips
        .iter()
        .zip(results)
        .map(|(c, r)| format!("{}\t{}\t{}\n", c.file, c.reference, r.hypothesis))
        .collect();
    let path = dir.join(format!("{engine}-{set}.tsv"));
    std::fs::write(&path, format!("file\treference\thypothesis\n{rows}"))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn score(clips: &[Clip], results: &[ClipResult]) -> Value {
    let pairs = clips
        .iter()
        .zip(results)
        .map(|(c, r)| (c.reference.as_str(), r.hypothesis.as_str()));
    let rtf: Vec<f64> = clips
        .iter()
        .zip(results)
        .map(|(c, r)| r.processing.as_secs_f64() / c.secs())
        .collect();
    json!({
        "wer": round(wer::corpus_wer(pairs), 4),
        "rtf_mean": round(mean(&rtf), 4),
        "rtf_p95": round(p95(&rtf), 4),
        "clips": clips.len(),
    })
}

/// Fast cleanup latency per transcript, with the seed dictionary.
fn bench_fast_cleanup(transcripts: &[String]) -> Value {
    let dictionary = Dictionary::parse(SEED);
    let per_call_ms: Vec<f64> = transcripts
        .iter()
        .map(|t| {
            let start = Instant::now();
            for _ in 0..CLEANUP_REPS {
                std::hint::black_box(fastclean::clean(std::hint::black_box(t), &dictionary));
            }
            start.elapsed().as_secs_f64() * 1000.0 / f64::from(CLEANUP_REPS)
        })
        .collect();
    json!({
        "mode": "fast",
        "inputs": "Parakeet transcripts of dictation_10",
        "latency_ms_mean": round(mean(&per_call_ms), 4),
        "latency_ms_p95": round(p95(&per_call_ms), 4),
        "calls": per_call_ms.len(),
    })
}

/// Smart cleanup latency per transcript through the app's own `smart_clean`,
/// with the server already started (the app starts it while the user talks)
/// and the seed dictionary. A call that falls back to Fast still counts, at
/// the time it took, and its note is kept.
fn bench_smart_cleanup(app_data: &Path, transcripts: &[String]) -> Value {
    let not_measured = |reason: String| json!({ "mode": "smart", "model": SMART_MODEL, "status": "not measured", "reason": reason });
    let bin = llama_server_path(&Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor"));
    if !bin.is_file() {
        return not_measured(format!(
            "{} is missing: run `npm run fetch-llama` in app/",
            bin.display()
        ));
    }
    let Some(model) = llm::model_path(app_data, SMART_MODEL).filter(|m| m.is_file()) else {
        return not_measured(format!(
            "{SMART_MODEL} is not downloaded: cargo run --example fetch_models {SMART_MODEL}"
        ));
    };
    let model_file = model.file_name().map(|n| n.to_string_lossy().into_owned());

    let engine = LlamaServerEngine::new(bin, model, None);
    let start = Instant::now();
    engine.warm();
    while engine.status() == EngineState::Loading {
        assert!(
            start.elapsed() < LOAD_TIMEOUT,
            "llama-server didn't start within {LOAD_TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let server_start_ms = start.elapsed().as_secs_f64() * 1000.0;
    if let EngineState::Failed { reason } = engine.status() {
        return not_measured(format!("llama-server failed to start: {reason}"));
    }

    let dictionary = Dictionary::parse(SEED);
    let terms = dictionary.llm_terms();
    let clean = |raw: &str| {
        smart_clean(
            &*engine,
            raw,
            &terms,
            StructureMode::None,
            SMART_BUDGET,
            || fastclean::clean(raw, &dictionary),
        )
    };
    clean(&transcripts[0]);
    let (mut per_call_ms, mut fallbacks) = (Vec::new(), Vec::new());
    for _ in 0..SMART_REPS {
        for raw in transcripts {
            let t = Instant::now();
            let result = clean(raw);
            per_call_ms.push(t.elapsed().as_secs_f64() * 1000.0);
            if !result.used_llm {
                fallbacks.push(result.note.unwrap_or_default());
            }
        }
    }
    engine.shutdown();

    json!({
        "mode": "smart",
        "model": SMART_MODEL,
        "model_file": model_file,
        "server": "llama.cpp llama-server, the build pinned in scripts/fetch-llama.mjs",
        "inputs": format!("Parakeet transcripts of dictation_10, {SMART_REPS} passes after one warm-up call"),
        "budget_ms": SMART_BUDGET.as_millis(),
        "server_start_ms": round(server_start_ms, 0),
        "latency_ms_mean": round(mean(&per_call_ms), 1),
        "latency_ms_p95": round(p95(&per_call_ms), 1),
        "calls": per_call_ms.len(),
        "fell_back_to_fast": fallbacks.len(),
        "fallback_notes": fallbacks,
    })
}

fn machine(repo: &Path) -> Value {
    let cpu_cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    json!({
        "cpu": cpu_name(),
        "cores": cpu_cores,
        "ram_gb": ram_bytes().map(|b| b / (1 << 30)),
        "os": os_name(),
        "arch": std::env::consts::ARCH,
        "load_avg_1m_at_start": load_avg(),
        "purl_commit": run(Command::new("git").arg("-C").arg(repo).args(["rev-parse", "--short", "HEAD"])),
    })
}

fn cpu_name() -> Option<String> {
    if cfg!(target_os = "macos") {
        run(Command::new("sysctl").args(["-n", "machdep.cpu.brand_string"]))
    } else if cfg!(windows) {
        run(Command::new("wmic").args(["cpu", "get", "name", "/value"]))
            .map(|s| s.trim_start_matches("Name=").to_string())
    } else {
        None
    }
}

fn ram_bytes() -> Option<u64> {
    if cfg!(target_os = "macos") {
        run(Command::new("sysctl").args(["-n", "hw.memsize"]))?
            .parse()
            .ok()
    } else if cfg!(windows) {
        run(Command::new("wmic").args(["computersystem", "get", "TotalPhysicalMemory", "/value"]))?
            .trim_start_matches("TotalPhysicalMemory=")
            .parse()
            .ok()
    } else {
        None
    }
}

/// Other work on the machine skews the speed numbers, so the run says how
/// busy it was.
fn load_avg() -> Option<f64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    // "{ 1.23 2.34 3.45 }"
    run(Command::new("sysctl").args(["-n", "vm.loadavg"]))?
        .trim_matches(|c| c == '{' || c == '}' || c == ' ')
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn os_name() -> Option<String> {
    if cfg!(target_os = "macos") {
        let version = run(Command::new("sw_vers").arg("-productVersion"))?;
        Some(format!("macOS {version}"))
    } else if cfg!(windows) {
        run(Command::new("cmd").args(["/C", "ver"]))
    } else {
        None
    }
}

/// A command's trimmed stdout, or None if it failed.
fn run(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn total_secs(clips: &[Clip]) -> f64 {
    clips.iter().map(Clip::secs).sum()
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len().max(1) as f64
}

/// Nearest-rank 95th percentile.
fn p95(xs: &[f64]) -> f64 {
    let mut sorted = xs.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((0.95 * sorted.len() as f64).ceil() as usize).max(1);
    sorted.get(rank - 1).copied().unwrap_or(0.0)
}

fn ms(d: Duration) -> f64 {
    round(d.as_secs_f64() * 1000.0, 1)
}

fn round(x: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (x * f).round() / f
}

/// "Apple M1 Pro" → "apple-m1-pro".
fn slug(s: &str) -> String {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p95_is_nearest_rank() {
        let xs: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(p95(&xs), 19.0);
        assert_eq!(p95(&[3.0]), 3.0);
        assert_eq!(p95(&[]), 0.0);
    }

    #[test]
    fn reads_whisper_timings() {
        let log = "whisper_print_timings:     load time =    68.31 ms\n\
                   whisper_print_timings:    total time =   284.95 ms\n";
        assert_eq!(
            timing(log, "load time"),
            Some(Duration::from_micros(68_310))
        );
        assert_eq!(
            timing(log, "total time"),
            Some(Duration::from_micros(284_950))
        );
        assert_eq!(timing(log, "encode time"), None);
    }

    #[test]
    fn slug_names_the_machine() {
        assert_eq!(slug("Apple M1 Pro"), "apple-m1-pro");
    }
}
