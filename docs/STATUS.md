# Purl v1 — status

_Last updated 2026-10-10 (end of day)._ Plan: [`docs/plans/2026-10-09-purl-v1.md`](plans/2026-10-09-purl-v1.md) ·
Spec: [`docs/specs/2026-10-09-purl-design.md`](specs/2026-10-09-purl-design.md)

Purl began as VendastaTalk v0.8.0, an earlier internal prototype, and was imported with fresh history.

## Done (merged)

| Task | What | PR |
|---|---|---|
| 0 | Repositories created | — |
| 1 | Prototype imported as Purl, rebranded, fresh history | #1 |
| 2 | BYOK, Ollama and cloud cleanup removed; Off / Fast / Smart modes; untrusted-text cleaning for LLM prompts | #3 |
| 4 | Model registry with pinned sha256; resumable, verified downloads; disk-space check | #2 |
| 4 (hardening) | 416 restart, size cap, panic reporting, `kind` on `model-done`, cancel is not a failure | #7 |
| 8 | First design system (superseded by Task 8b) | #4 |
| 9 | Onboarding: permissions, in-app speech-model download, try-it step, 90-second field-reading disclosure | #6 |
| 10 | All screens and Settings rebuilt; arrow-key radio groups; disclosure text from one source | #9 |
| — | Spec/plan amendments: no Intel Mac in v1, "Purl" in the dictionary, motion-pack design, random LLM fences | #8, #11 |
| 8b (site) | Shared design system from the motion pack (landing repo) | landing repo |

## In review / ready for review

| Task | What | PR | State |
|---|---|---|---|
| 3 | Platform layer, Windows build, CI on macOS + Windows | #5 | fixes verified; merge-only re-review, then merge first |
| 5 | Parakeet v3 speech-to-text in-process; whisper.cpp removed | #12 | fixes verified; needs main merged in (conflicts after #9) |
| 7 | Release pipeline: Apple Silicon DMG, Windows installer, updater manifest, smoke tests | #10 | draft; after #5 |
| 8b (app) | App on the motion design: live dictation pill, Home voice orb, every screen | #14 | awaiting review |
| — | This status page | #13 | reviewed |
| 14 | Landing page on the motion design | landing repo | awaiting review |

## In progress (no PR yet)

- **Task 6 — Smart cleanup:** bundled llama-server + Qwen3.5 2B, output guard, transforms, random per-request
  prompt fences, server identity checks. Opens as a PR after #12.
- **Task 11 — benchmark:** done on a branch; opens after #12.
- **Task 12 — QA matrix:** written; hand run waits for #5, #12 and Task 6.

## Not started

- Task 13 README, guide, privacy, case study · Task 15 v1.0.0 release.

## Measured so far (Apple M1 Pro)

| What | Result |
|---|---|
| Parakeet v3 download | 639 MiB |
| Parakeet first transcription (2.5 s clip) | 0.8–1.2 s |
| Parakeet warm transcription (2.5 s clip) | 117–178 ms |
| Word error rate, dictation clips (synthetic) | Parakeet 0.81% · whisper.cpp base.en 1.62% |
| Word error rate, LibriSpeech test-clean (100 clips) | Parakeet 3.20% · whisper.cpp base.en 2.86% |

The word-error numbers come from the Task 11 benchmark branch (not merged yet). Parakeet's LibriSpeech score is
held back by a repetition bug on 3 of 100 clips, which is being fixed. Smart cleanup speed is not measured yet.

## Known gaps

- Nothing has run on real Windows hardware yet; Windows coverage is CI build, tests and an install smoke test.
- Installers are unsigned on Windows and self-signed on macOS in v1.
- Intel Macs are not supported in v1.
