# Purl v1 — status

_Last updated 2026-10-10._ Plan: [`docs/plans/2026-10-09-purl-v1.md`](plans/2026-10-09-purl-v1.md) ·
Spec: [`docs/specs/2026-10-09-purl-design.md`](specs/2026-10-09-purl-design.md)

## Done (merged)

| Task | What | PR |
|---|---|---|
| 1 | VendastaTalk v0.8.0 imported as Purl, rebranded, fresh history | #1 |
| 2 | BYOK, Ollama and cloud cleanup removed; Off / Fast / Smart modes; untrusted-text cleaning for LLM prompts | #3 |
| 4 | Model registry with pinned sha256; resumable, verified downloads; disk-space check | #2 |
| 4 (hardening) | 416 restart, size cap, panic reporting, `kind` on `model-done`, cancel is not a failure | #7 |
| 8 | First design system (since superseded by Task 8b) | #4 |
| 9 | Onboarding: permissions, in-app speech-model download, try-it step, 90-second field-reading disclosure | #6 |
| — | Spec/plan amendments: no Intel Mac in v1, dictionary seeded with "Purl" | #8 |

## In review

| Task | What | PR |
|---|---|---|
| 3 | Platform layer (`Trust`, text injection, focus reader), Windows build, CI on macOS + Windows | #5 |
| 5 | Parakeet v3 speech-to-text in-process via transcribe-rs; whisper.cpp removed | #12 |
| 7 | Release pipeline: Apple Silicon DMG, Windows NSIS installer, updater manifest, smoke tests | #10 (draft, stacked on #5) |
| 10 | All screens and Settings rebuilt (review fixes in progress) | #9 |
| — | Spec/plan: motion-pack design direction, random LLM prompt fences, `model-done.kind` | #11 |

## In progress (no PR yet)

- **Task 6 — Smart cleanup:** bundled llama-server + Qwen3.5 2B, output guard, transforms. Adding random
  per-request prompt fences and a server identity check before review.
- **Task 8b — design:** shared design system from the motion pack, then the landing page and the app on it.
- **Task 11 — benchmark:** Parakeet vs whisper.cpp base.en (accuracy and speed) on LibriSpeech and dictation clips.

## Not started

- Task 12 QA matrix · Task 13 README, guide, privacy, case study · Task 15 v1.0.0 release.

## Measured so far (Apple M1 Pro)

| What | Result |
|---|---|
| Parakeet v3 download | 639 MiB |
| Parakeet first transcription (load + decode, 2.5 s clip) | 0.8–1.2 s |
| Parakeet warm transcription (2.5 s clip) | 120–180 ms |
| Smart cleanup, Qwen3.5 2B (short dictation) | 155–750 ms |

These are spot checks, not the benchmark. The reproducible benchmark (Task 11) replaces them.

## Known gaps

- Nothing has run on real Windows hardware yet; Windows coverage is CI build, tests and an install smoke test.
- Installers are unsigned on Windows and self-signed on macOS in v1.
- Intel Macs are not supported in v1.
