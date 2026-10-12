# Purl v1 — status

_Last updated 2026-10-12._ Plan: [`docs/plans/2026-10-09-purl-v1.md`](plans/2026-10-09-purl-v1.md) ·
Spec: [`docs/specs/2026-10-09-purl-design.md`](specs/2026-10-09-purl-design.md)

## Done (merged)

| Task | What | PR |
|---|---|---|
| 0 | Repositories created | — |
| 1 | Earlier prototype imported as Purl, rebranded, fresh history | #1 |
| 2 | BYOK, Ollama and cloud cleanup removed; Off / Fast / Smart modes; untrusted-text cleaning for LLM prompts | #3 |
| 3 | Platform layer, Windows build, CI on macOS + Windows | #5 |
| 4 | Model registry with pinned sha256; resumable, verified downloads; disk-space check | #2 |
| 4 (hardening) | 416 restart, size cap, panic reporting, `kind` on `model-done`, cancel is not a failure | #7 |
| 5 | Parakeet v3 speech-to-text in-process; whisper.cpp removed | #12 |
| 6 | Smart cleanup: bundled llama-server + Qwen3.5 2B, output guard, transforms, random prompt fences, server identity check | #16 |
| 7 | Release pipeline: Apple Silicon DMG, Windows installer, updater manifest, smoke tests, [release guide](RELEASING.md) | #10 |
| 8 | First design system (superseded by Task 8b) | #4 |
| 8b | Shared design system from the motion pack; the app on it: live dictation pill, Home voice orb, every screen | #14 |
| 9 | Onboarding: permissions, in-app speech-model download, try-it step, 90-second field-reading disclosure | #6 |
| 10 | All screens and Settings rebuilt; disclosure text from one source | #9 |
| 11 | Benchmark: Parakeet vs whisper.cpp base.en, Fast and Smart cleanup ([how to run](../bench/README.md)) | #17 |
| 14 | Landing page on the motion design (private repo, not deployed) | landing repo |
| — | Spec/plan amendments: no Intel Mac in v1, "Purl" in the dictionary, motion-pack design, random LLM fences | #8, #11 |

## Open

| Task | What | PR |
|---|---|---|
| 5 (fix) | Parakeet decoder fix for a token-repeat bug; re-measuring | not opened yet |
| — | Review follow-ups: Smart sidecar hardening | #18 |
| 13 | README, guide, privacy page, case study | #19 |
| — | This status page | #15 |

## Still to do

- **Task 12 — QA:** work the edge-case matrix by hand on a release build (macOS), plus the CI smoke on Windows.
- **Task 15 — v1.0.0 release:** needs the owner's signing certificate and updater key set as repository
  secrets (see [RELEASING.md](RELEASING.md)), then a tagged build, an install check and publishing.

## Benchmark (Apple M1 Pro)

From [`bench/results/2026-10-12-apple-m1-pro.json`](../bench/results/2026-10-12-apple-m1-pro.json). Apple M1 Pro, 10 cores,
16 GB, macOS 26.4.1. The machine was busy during the run (load average 18 to 26).

| What | Parakeet v3 (Purl) | whisper.cpp base.en |
|---|---|---|
| Word error rate, LibriSpeech test-clean (100 clips) | 3.2% | 2.9% |
| Word error rate, dictation clips (10, synthetic: read by macOS `say`) | 0.8% | 1.6% |
| Real-time factor, LibriSpeech, mean (lower is faster) | 0.041 (CPU) | 0.026 (Metal GPU) · 0.350 (CPU) |
| Model load, cold | 1101 ms | 81 ms (GPU) · 68 ms (CPU) |

| Cleanup | Mean | p95 |
|---|---|---|
| Fast | 0.0031 ms | 0.0037 ms |
| Smart, Qwen3.5 2B (30 calls, 0 fell back to Fast) | 910 ms | 983 ms |

The first Smart server start took 19.1 s. Windows: not measured yet.

Parakeet's LibriSpeech score is held back by a decoder bug that repeats a token: it adds 27 extra words out of
2,346, or 1.15 points of word error rate. A fix is being re-measured; the score above will change.

## Known gaps

- Nothing has run on real Windows hardware yet; Windows coverage is CI build, tests and an install smoke test.
- Installers are unsigned on Windows and self-signed on macOS in v1.
- Intel Macs are not supported in v1.
