# Parakeet accuracy after the decoder fix (2026-10-12, Apple M1 Pro)

Per-clip Parakeet transcripts from before and after PR #22, which makes the
decoder use Parakeet's TDT duration head. Columns: `file`, `reference`,
`hypothesis`.

- `after/`: written by `cargo run --release --example bench` on 2026-10-12,
  Apple M1 Pro, at the benchmark branch (`7c94447`) with PR #22's
  transcribe-rs patch. That run crashed later, in the whisper CPU-only
  pass (a whisper-cli timeout, machine load about 144), so it wrote no
  results JSON and its speed numbers aren't reported anywhere.
- `before/`: the cached Parakeet transcripts from before the fix. They score
  exactly what [`../2026-10-12-apple-m1-pro.json`](../2026-10-12-apple-m1-pro.json)
  reports.

Scored with the bench's own `corpus_wer` (`app/src-tauri/examples/bench/wer.rs`):

| Set | Before | After |
|---|---|---|
| LibriSpeech test-clean, 100 clips | 75 / 2,346 words = 3.20% | 51 / 2,346 = 2.17% |
| dictation_10 | 2 / 247 = 0.81% | 2 / 247 = 0.81% (same transcripts) |

The 3 clips that read "Saint Francis X X X X X X X X X Xavier" drop from 28
errors to 1. The fix also changes 8 other clips a little: 5 get worse (+6
errors), 3 get better (−3). PR #22 lists the words.

LibriSpeech is by Vassil Panayotov, Guoguo Chen, Daniel Povey and Sanjeev
Khudanpur ([openslr.org/12](https://www.openslr.org/12)), CC-BY-4.0.
