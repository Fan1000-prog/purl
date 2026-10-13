# Purl benchmark

How accurate and how fast is Purl's speech engine? This folder measures it,
the same way every time, so anyone can check the numbers.

It compares:

- **Parakeet v3** (`parakeet-tdt-0.6b-v3`, int8 ONNX): Purl's engine, run
  through the app's own code.
- **whisper.cpp base.en**: the engine Purl used before. It runs here through
  `whisper-cli` only for the comparison. It is not part of the app.

And it times both cleanups on the dictation transcripts: **Fast** (the
rule-based tidy-up) and **Smart** (Qwen3.5-2B on the bundled llama-server,
through the app's own `smart_clean`).

Results live in [`results/`](results/), one JSON file per run, named
`<date>-<cpu>.json`.

## Reproduce

You need: Rust, `curl`, `ffmpeg` (or macOS `afconvert`), and `whisper-cli`
(`brew install whisper-cpp` on macOS).

```bash
# 1. Purl's speech model (~670 MB) and the Smart model (~1.4 GB) into the
#    app-data dir, if the app hasn't already, and the pinned llama-server.
cd app/src-tauri
cargo run --example fetch_models
cargo run --example fetch_models qwen3.5-2b
(cd .. && npm run fetch-llama)

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
- **Cleanup latency**: time per cleanup call on Parakeet's 10 dictation
  transcripts, with the default dictionary. Fast: each timed over 1,000
  calls. Smart: three passes over the 10 after one untimed warm-up call,
  with the server already running (the app starts it while you talk) and
  the app's 6-second budget. Calls that fell back to Fast are counted and
  their reasons kept. **Server start** is the first start of llama-server,
  which the app does ahead of time when Smart is on.

Whisper is measured twice: with Metal GPU (whisper-cli's default on a Mac)
and CPU only (`-ng`), both with one thread per core. Parakeet runs on the CPU
through ONNX Runtime, as in the app.

## Machines

- macOS, Apple M1 Pro (10 cores, 16 GB): [`results/2026-10-10-apple-m1-pro.json`](results/2026-10-10-apple-m1-pro.json).
  The machine got busy during that run (load 2.9 at start, 40.6 at the end).
  whisper CPU-only, measured last, had a mean RTF of 0.18 on dictation_10
  and 0.36 on LibriSpeech, so read that row as rough.
- macOS, Apple M1 Pro, again: [`results/2026-10-12-apple-m1-pro.json`](results/2026-10-12-apple-m1-pro.json),
  the first run with Smart cleanup. Load was 18 at start and 26 at the end,
  mostly Spotlight indexing (CPU use stayed low). WER is identical to the
  earlier run. Speeds moved: Parakeet mean RTF up 3-4%, whisper-Metal up
  to 11%, Parakeet cold load 828 → 1,101 ms, and whisper CPU-only again
  noisy (0.31 mean RTF on dictation_10, was 0.18).
- Both runs above predate a Parakeet decoder fix
  ([PR #22](https://github.com/Fan1000-prog/purl/pull/22)): the decoder
  ignored the TDT duration head and wrote "Francis X X X X X X X X X Xavier"
  on 3 LibriSpeech clips. Re-scored after the fix, on the same machine on
  2026-10-12: LibriSpeech 3.20% → 2.17% (75 → 51 of 2,346 words),
  dictation_10 unchanged at 0.81%. That run crashed in the whisper CPU-only
  pass, so there's no JSON and no new speed numbers; its per-clip
  transcripts are in
  [`results/2026-10-12-apple-m1-pro-tdt-fix/`](results/2026-10-12-apple-m1-pro-tdt-fix/).
- Windows numbers: not measured yet.
