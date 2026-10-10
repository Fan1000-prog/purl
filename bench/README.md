# Purl benchmark

How accurate and how fast is Purl's speech engine? This folder measures it,
the same way every time, so anyone can check the numbers.

It compares:

- **Parakeet v3** (`parakeet-tdt-0.6b-v3`, int8 ONNX): Purl's engine, run
  through the app's own code.
- **whisper.cpp base.en**: the engine Purl used before. It runs here through
  `whisper-cli` only for the comparison. It is not part of the app.

And it times **Fast cleanup** (the rule-based tidy-up) on the dictation
transcripts. **Smart cleanup** (Qwen3.5-2B) is listed as "not measured yet"
until it lands.

Results live in [`results/`](results/), one JSON file per run, named
`<date>-<cpu>.json`.

## Reproduce

You need: Rust, `curl`, `ffmpeg` (or macOS `afconvert`), and `whisper-cli`
(`brew install whisper-cpp` on macOS).

```bash
# 1. Purl's speech model (~670 MB) into the app-data dir, if the app hasn't already.
cd app/src-tauri
cargo run --example fetch_models

# 2. Test data and the whisper baseline model into ~/Library/Caches/purl-bench
#    (~350 MB download; nothing lands in the repo).
../../bench/fetch.sh

# 3. Run. Prints the JSON and writes bench/results/<date>-<cpu>.json.
cargo run --release --example bench
```

Options: `--cache DIR` (or `PURL_BENCH_CACHE`), `--model-dir DIR`,
`--whisper-cli PATH`, `--out FILE`. Per-clip transcripts for each engine are
saved to `<cache>/runs/` so you can see every error.

Close other heavy apps first. Speed numbers move a lot when the machine is
busy, so each result records `load_avg_1m_at_start` and `load_avg_1m_at_end`.

## Test sets

| Set | What | Where |
| --- | --- | --- |
| `librispeech_100` | LibriSpeech `test-clean`, the first 100 utterances by sorted id (15 min of read audiobook speech). [openslr.org/resources/12](https://www.openslr.org/12), CC-BY-4.0. | Downloaded by `fetch.sh` into the cache. Not committed. |
| `dictation_10` | 10 dictation-style clips: emails, Google Chat messages, notes. **Synthetic**: read by macOS `say` (voices Samantha and Daniel), not recorded by a person. | [`dictation/`](dictation/), text in `manifest.tsv` |

Synthetic speech is clean and evenly paced, so `dictation_10` is easier than
real dictation. Treat it as a check that everyday office words come out
right, not as a real-world accuracy number.

## What the numbers mean

- **WER** (word error rate): wrong, missing and extra words divided by the
  number of words spoken, over the whole set. Lower is better. 0.03 = 3 words
  in 100 wrong. Before scoring, text is lowercased and punctuation is
  dropped; hyphens split words. Nothing else is normalized, so these all
  count as errors: `ten` vs `10`, `colour` vs `color`, `saint` vs `st`.
  Published WER figures usually use a heavier normalizer, so they come out
  lower than these.
- **RTF** (real-time factor): time to transcribe a clip divided by the clip's
  length, with the model already loaded. Lower is better. 0.05 = a 10-second
  clip takes half a second. Reported as the mean and the 95th percentile
  over the clips.
- **Cold load**: time to load the model in a fresh process. The model files
  may already be in the OS file cache from earlier runs, so a first launch
  after a reboot can be slower. For whisper this is whisper-cli's own
  reported load time.
- **Cleanup latency**: time per Fast cleanup call on Parakeet's 10 dictation
  transcripts (each timed over 1,000 calls), with the default dictionary.

Whisper is measured twice: with Metal GPU (whisper-cli's default on a Mac)
and CPU only (`-ng`), both with one thread per core. Parakeet runs on the CPU
through ONNX Runtime, as in the app.

## Machines

- macOS, Apple M1 Pro (10 cores, 16 GB): [`results/2026-10-10-apple-m1-pro.json`](results/2026-10-10-apple-m1-pro.json).
  The machine got busy during that run (load 2.9 at start, 40.6 at the end).
  Parakeet and whisper-Metal speeds matched two other runs within a few
  percent; whisper CPU-only, measured last, ranged from 0.18 to 0.36 mean RTF
  across runs, so read that row as rough.
- Windows numbers: not measured yet.
