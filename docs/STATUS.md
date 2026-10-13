# Purl v1 — status

_Last updated 2026-10-13._ Plan: [`docs/plans/2026-10-09-purl-v1.md`](plans/2026-10-09-purl-v1.md) ·
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
| 5 (fix) | Parakeet no longer repeats one token: transcribe-rs pinned to a fork that uses the TDT duration head | #22 |
| 6 | Smart cleanup: bundled llama-server + Qwen3.5 2B, output guard, transforms, random prompt fences, server identity check | #16 |
| 6 (hardening) | Smart sidecar: reap by full path, atomic release | #18 |
| 7 | Release pipeline: Apple Silicon DMG, Windows installer, updater manifest, smoke tests, [release guide](RELEASING.md) | #10 |
| 7 (smoke) | Release smoke tests run the bundled llama-server on both OSes; Windows tar fix. A dry run passed on both. | #25 |
| 8 | First design system (superseded by Task 8b) | #4 |
| 8b | Shared design system from the motion pack; the app on it: live dictation pill, Home voice orb, every screen | #14 |
| 9 | Onboarding: permissions, in-app speech-model download, try-it step, 90-second field-reading disclosure | #6 |
| 10 | All screens and Settings rebuilt; disclosure text from one source | #9 |
| 10 (hotkey) | The app says "hold to talk" and tells you when a tap is too short | #23 |
| 10 (hotkey) | A hotkey tap stops a recording started from the button or tray, instead of discarding it | #28 |
| 11 | Benchmark: Parakeet vs whisper.cpp base.en, Fast and Smart cleanup ([how to run](../bench/README.md)) | #17 |
| 13 | README, guide, privacy page, case study | #19 |
| — | Review follow-ups: app UI nits, pipeline nits, inject and comment nits, a Windows download-test flake, script symlinks | #20, #21, #24, #26, #27, #29 |
| — | Spec/plan amendments: no Intel Mac in v1, "Purl" in the dictionary, motion-pack design, random LLM fences | #8, #11 |
| 14 | Landing page on the motion design. The code is in the landing repo; it isn't hosted anywhere now. | landing repo |

## Open

| What | PR |
|---|---|
| Docs: Parakeet accuracy after the decoder fix (README, case study, per-clip evidence) | #30 |
| This status page | #15 |

## Left for v1.0.0

1. **Signing secrets (owner only).** Create the "Purl Self-Signed" certificate and set the three repository
   secrets: `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`, `TAURI_SIGNING_PRIVATE_KEY`. Back up the `.p12`
   and the updater key. Steps: [RELEASING.md](RELEASING.md#one-time-setup-signing-secrets).
2. **Version bump** from 0.9.0 to 1.0.0 in all three places.
3. **Dry run**, then tag a release candidate.
4. **Install test** from the downloaded DMG, including Smart cleanup: the bundled llama-server keeps
   upstream's ad-hoc signature, so it has to be checked from a real download.
5. **Task 12 QA:** work the edge-case matrix by hand on that release build.
6. **Tag and publish v1.0.0.**

Known open item: in the benchmark, a whisper.cpp timeout stops the whole run. Fixing it needs a scoring
decision (drop the clip from the score, or count it all wrong).

## Benchmark (Apple M1 Pro)

From [`bench/results/2026-10-12-apple-m1-pro.json`](../bench/results/2026-10-12-apple-m1-pro.json), Apple M1 Pro,
10 cores, 16 GB, macOS 26.4.1. The machine was busy during the run (load average 18 to 26).

| What | Parakeet v3 (Purl) | whisper.cpp base.en |
|---|---|---|
| Word error rate, LibriSpeech test-clean (100 clips) | 2.2% after the decoder fix (3.2% before) | 2.9% |
| Word error rate, dictation clips (10, synthetic: read by macOS `say`) | 0.8% | 1.6% |
| Real-time factor, LibriSpeech, mean (lower is faster) | 0.041 (CPU) | 0.026 (Metal GPU) · 0.350 (CPU) |
| Model load, cold | 1101 ms | 81 ms (GPU) · 68 ms (CPU) |

| Cleanup | Mean | p95 |
|---|---|---|
| Fast | 0.0031 ms | 0.0037 ms |
| Smart, Qwen3.5 2B (30 calls, 0 fell back to Fast) | 910 ms | 983 ms |

The first Smart server start took 19.1 s. Windows: not measured yet.

The 2.2% is 51 errors in 2,346 words, scored with the bench's own scorer on that run's Parakeet transcripts
with the fix applied (#22). The fix doesn't change the dictation clips. A full re-run stopped on a whisper.cpp
timeout, so there is no new results file yet. Speed and Smart timings are from before the fix and weren't
re-measured.

## Known gaps

- Nothing has run on real Windows hardware yet; Windows coverage is CI build, tests and an install smoke test.
- Installers are unsigned on Windows and self-signed on macOS in v1.
- Intel Macs are not supported in v1.
