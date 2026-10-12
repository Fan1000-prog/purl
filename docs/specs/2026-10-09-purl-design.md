# Purl — design

**Date:** 2026-10-09
**Status:** design approved in chat 2026-10-09; this file awaits review
**Origin:** VendastaTalk v0.8.0 (`Citizen_Developer_Projects/Users/fani-bango/Vendasta-Talk`), Tauri v2, ~6k lines Rust + ~2.5k lines TS
**Brand:** `~/Desktop/Purl-Design/02-motion` motion pack + `01-logo-locked` logo (amended 2026-10-10)

## Goal

Ship **Purl**: a free, open-source (MIT), local-first dictation app for macOS and
Windows. Press a hotkey, talk, press again — the words are transcribed on-device,
cleaned up, and pasted at the cursor. No account, no API keys, no audio leaves
the machine.

## Success criteria

1. A new user on macOS (Apple Silicon) or Windows 10/11 x64 installs Purl
   from the README in under 5 minutes, with no Homebrew, no Ollama, no terminal on
   Windows.
2. Hotkey → speak → cleaned text pasted into Chrome (Gmail, Google Chat), Notepad /
   TextEdit and VS Code, on both platforms.
3. Zero outbound network calls at dictation time. The only network use is model
   downloads (Hugging Face / GitHub Releases) and the update check.
4. CI builds and tests green on `macos-latest` and `windows-latest` on every PR.
5. README benchmark numbers are measured on real hardware, reproducible from a
   script in the repo, and state the machine they ran on.

## Non-goals (v1)

- Any hosted service, account, quota or billing. BYOK is removed, not hidden.
- Correction learning on Windows (UI Automation reader). Ships disabled on Windows
  with a clear note; macOS keeps it.
- Linux, Windows on ARM, mobile.
- **Intel Macs** (amended 2026-10-10). ONNX Runtime ≥1.27, which `ort`/`transcribe-rs` need, has no official
  x86_64-apple-darwin build (Microsoft's last Intel Mac release is 1.23; pyke ships none). Building it ourselves is
  deferred until users ask. README and landing page say Intel Macs aren't supported yet.
- Authenticode signing and Apple notarization. macOS builds use a stable self-signed identity so the Accessibility grant survives updates. v1 Windows installer is unsigned; the guide covers the
  SmartScreen "More info → Run anyway" step. Signing is the first v1.x item.
- Streaming / live preview transcription.

## Deliverables

| # | Deliverable | Where |
|---|---|---|
| 1 | App source, MIT, fresh git history | public `github.com/Fan1000-prog/purl`, local `~/Desktop/purl` |
| 2 | Releases: macOS `.dmg` for Apple Silicon only (amended 2026-10-10: no Intel build in v1, see Non-goals), Windows NSIS `.exe` (x64), `latest.json` for the updater | GitHub Releases on `purl` |
| 3 | README: pitch, install (both OSes), guide, benchmark, case study, privacy, credits/licences | `purl/README.md` + `purl/docs/` |
| 4 | Landing page | private `github.com/Fan1000-prog/purl-site`, local `~/Desktop/purl-site`, hosted on Vercel (deploy needs a go-ahead) |

**Fresh history.** The VendastaTalk history carries internal Vendasta context
(jargon glossary, internal URLs, staff names). The new repo starts with one
import commit, then normal history. The case study tells the backstory instead.

## Architecture

One Tauri v2 app. Rust core does everything except drawing; the webview draws.

```
purl/
  app/
    src/                     # TS + Vite UI (redesigned)
    src-tauri/src/
      engine/                # NEW: sidecar lifecycle + model registry
        mod.rs               #   traits, EngineStatus, SidecarProcess (llama-server only)
        models.rs            #   registry: id, url, sha256, size, licence
        download.rs          #   resumable, checksum-verified, disk-space-checked
        stt.rs               #   transcribe-rs Parakeet, in-process
        llm.rs               #   llama-server client (Qwen), OpenAI-compatible
      platform/              # from the Windows port spec
        mod.rs  mac.rs  windows.rs
      cleanup.rs fastclean.rs transforms.rs voice_commands.rs snippets.rs
      corrections.rs learned.rs ax_read.rs (mac only) history.rs stats.rs
      dictionary.rs          # was glossary.rs, user-owned; ships with one entry, "Purl"
      audio.rs settings.rs lib.rs
```

### Layers and contracts

```
UI (TS)  ──invoke/events──▶  lib.rs commands
                               │
          ┌────────────────────┼──────────────────────┐
          ▼                    ▼                      ▼
     pipeline             platform traits         engine
  (cleanup, transforms,  (TextInjector,          (SttEngine, LlmEngine,
   commands, snippets,    FocusReader,            ModelStore)
   corrections)           EngineLocator)
```

- **Pipeline** is platform-blind and engine-blind: it takes `&dyn SttEngine` /
  `&dyn LlmEngine` and is tested with fakes.
- **STT** runs in-process. **LLM** runs as a `llama-server` sidecar spoken to over
  localhost HTTP only, bound to `127.0.0.1` on a random free port. Never `0.0.0.0`.
- **Platform** traits come from the 2026-08-14 Windows port spec, minus
  `SecretStore` (no secrets left to store).

```rust
pub trait SttEngine: Send + Sync {
    fn transcribe(&self, pcm16k_mono: &[f32], lang: Lang) -> Result<String, SttError>;
    fn status(&self) -> EngineStatus;          // Missing | Downloading{pct} | Starting | Ready | Failed{reason}
}
pub trait LlmEngine: Send + Sync {
    fn complete(&self, system: &str, user: &str, budget: Duration) -> Result<String, LlmError>;
    fn status(&self) -> EngineStatus;
}
```

`budget` is a hard deadline. On timeout or error the pipeline falls back to Fast
cleanup and pastes; it never blocks the paste on the LLM.

### Speech-to-text

- Runtime: **`transcribe-rs`** (MIT, `features = ["onnx"]`), in-process via ONNX
  Runtime (`ort`). Same approach as Handy (Tauri + Rust, MIT), proven on macOS and
  Windows. No sidecar process, no websocket, no DLL renaming. *(Amended
  2026-10-09; replaces the sherpa-onnx sidecar.)*
- Model: **parakeet-tdt-0.6b-v3 int8** in ONNX form from Hugging Face
  `istupakov/parakeet-tdt-0.6b-v3-onnx` — four files, ~670 MB
  (`encoder-model.int8.onnx`, `decoder_joint-model.int8.onnx`, `nemo128.onnx`,
  `vocab.txt`). 25 European languages, CC-BY-4.0 — attribution in README and
  About. Downloaded in onboarding.
- The loaded model stays resident between dictations and is dropped after 15 idle
  minutes; loading runs on a background thread so the UI never blocks.
- Long audio is segmented at ≤30 s on silence boundaries before decode.
- whisper.cpp, `whisper_server.rs` and Homebrew lookups are deleted.

### Cleanup

Two tiers, user-selectable in Settings:

1. **Fast** (default, no download): existing `fastclean.rs` + `cleanup.rs` rules —
   filler removal, punctuation, capitalisation, personal dictionary, snippets.
   The dictionary ships with one entry, `Purl` (amended 2026-10-10): Parakeet hears it as
   "Pearl". The entry fixes casing and biases Smart cleanup; Fast cleanup does not rewrite
   "pearl" → "Purl", because that would corrupt the real word.
2. **Smart** (opt-in): bundled **llama.cpp `llama-server`** (MIT) +
   **Qwen3.5-2B Q4_K_M GGUF** (~1.3 GB, Apache-2.0), downloaded when the
   user turns it on. A larger 4B option is offered, not default. The
   server is started on demand and stopped after 5 idle minutes.
   Cleanup prompt follows OpenWhispr's rule: *the speaker is never talking to you*
   — the transcript is wrapped in `<transcript>` tags and output is rejected (fall
   back to Fast) if it is empty, >1.5× or <0.4× the input length, or contains the
   tags. Transforms use the same engine.

Ollama detection, `ollama_pull.rs`, `cloud_llm.rs`, `secrets.rs` and the cloud-key
UI are deleted.

### Model downloads

Single `ModelStore` for both engines:

- Registry entries pin URL, size and **sha256**; a mismatch deletes the file and
  reports failure.
- Resumable (HTTP Range), into `<app_data>/models/<id>.part`, atomic rename on
  success.
- Pre-check free disk space ≥ size + 10%; refuse with a readable message.
- Offline / DNS failure / 4xx / 5xx each map to a distinct user-facing message
  with a Retry button.
- Models live in the OS app-data dir (`~/Library/Application Support/app.purl`,
  `%APPDATA%\app.purl`), never in the install dir.

### Platform

Per the Windows port spec: `TextInjector` (SendInput / CGEvent), `EngineLocator`
(bundled resource dir only, `.exe` suffix, `CREATE_NO_WINDOW`, Job Object so
sidecars die with the app on Windows; PID file + reap on macOS),
`FocusReader` (macOS AX via frontmost-app route; Windows returns `None` in v1).

The `Trust` enum (`Granted | Denied | NotRequired`) replaces the bool
`accessibility_trusted()` at every call site so Windows shows "ready" rather than
a permission prompt that leads nowhere.

### Identity and updates

- Bundle id `app.purl.desktop`; product name `Purl`.
- Updater endpoint: `github.com/Fan1000-prog/purl/releases/latest/download/latest.json`,
  with darwin-aarch64 and windows-x86_64 entries, signed with a new Tauri
  updater key (private key stays out of the repo; CI reads it from a GitHub secret).
- First launch does **not** migrate VendastaTalk settings or history. Separate
  app, separate data dir.

## Redesign

*Amended 2026-10-10 (second time): the direction is the user's motion pack at
`~/Desktop/Purl-Design/02-motion`, with the locked logo from `~/Desktop/Purl-Design/01-logo-locked`.
It replaces both the mascot kit and the interim "Brand v2" (Newsreader/Inter), which are rejected.*

- **Identity:** the locked speech-bubble **p** with the breath tick (bead + sound arc), used exactly as drawn.
  No faces, eyes or characters.
- **Palette:** Night `#12100E`, Ink `#1F1A17`, Deep `#2A231E`, Fern `#3F6B4B`, Fern Glow `#7BC48C`,
  Sprout `#B7CDA3`, Butter `#F3D36B`, Ember `#EC8A5A`, Terracotta `#C0683F`, Cream `#F7F1E8`.
  The **Purl Spectrum** (Fern Glow → Sprout → Butter → Ember → Terracotta) appears only in "polish" moments.
  Dark is the primary surface; cream is the light theme.
- **Type:** Instrument Serif (display, roman + italic), Geist (UI), Geist Mono (labels, timers). All OFL,
  bundled locally.
- **One design system** (`brand/` in purl-site: tokens, fonts, logo, live components) drives both the landing
  page and the app. The app vendors it. App contract tokens: `--bg --surface --text --muted --accent
  --accent-ink --warn --warn-text --line --font-display --font-ui --font-mono`.
- **Live components, not video, in the app:** the dictation pill (idle with the real hotkey → listening with a
  waveform driven by mic level + timer → polishing with spectrum border/shimmer → sent → error) and the voice
  orb (idle/listening/polishing/done). Static under `prefers-reduced-motion`.
- **Landing page:** uses the pack's hero, product demo, how-it-works and works-anywhere motion (WebM first,
  lazy-loaded, posters under reduced motion), with live HTML text for headings.
- **References:** the pack borrows *patterns* from wisprflow.ai and Heidi Dictate. No text, logos, footage or
  assets from either; Purl must not be mistakable for Wispr Flow (a competitor).
- Onboarding: 4 steps (welcome → mic (+ Accessibility on macOS) → speech model download with progress →
  try it). Smart cleanup is offered afterwards.
- Screens: Home (orb, recent + stats), Dictionary, Snippets, Transforms, Settings. Every screen has a designed
  empty state and error state.

## Error handling and edge cases (QA matrix)

| Case | Expected |
|---|---|
| No mic / mic permission denied | Onboarding and overlay say so with a link to OS settings; hotkey does nothing harmful |
| Mic unplugged mid-recording | Recording stops, captured audio is still transcribed, toast explains |
| Silence / <0.3 s audio | Nothing pasted, overlay shows "didn't catch that" |
| Very long recording (10 min) | Segmented decode, no OOM, progress visible |
| Model missing / corrupt / checksum fail | Re-download offered; dictation disabled with reason |
| Offline during download | Paused with Retry; resume picks up the `.part` |
| Disk full | Refused before download starts |
| Sidecar crash / port in use | Restart once with a new port; then Failed with "Copy diagnostics" |
| LLM slow / hung | Deadline hit → Fast cleanup result pasted |
| LLM returns junk / answers the prompt | Guard rejects → Fast cleanup |
| Focused field is a password field | No paste-read, no correction learning (macOS) |
| No focused text field | Text left on clipboard, toast says so |
| Hotkey conflict | Recorder shows the conflict, keeps the old binding |
| Two quick hotkey presses / press during processing | Debounced; one session at a time |
| Multi-monitor, mixed DPI | Overlay on the cursor's monitor, correct scale |
| App quit while sidecars run | Sidecars terminated; no orphans on next launch |
| Non-ASCII / accented speech | Round-trips through clipboard intact |
| Elevated target window (Windows UIPI) | Paste fails gracefully to clipboard + toast |

## Testing

- **Unit** (`cargo test`): pipeline with fake engines, registry, download
  (local HTTP fixture server: resume, checksum, 404, truncated), LLM output guard,
  `Trust` call sites, key mapping per platform.
- **Integration** (`cargo test -- --ignored`, CI nightly + before release):
  real transcribe-rs + Parakeet on fixture WAVs; real llama-server + Qwen on fixture
  transcripts.
- **Accuracy benchmark** (`bench/`): fixed public-domain clip set (LibriSpeech
  test-clean subset, CC-BY-4.0) plus 10 recorded dictation-style clips; reports
  WER and real-time factor for Parakeet v3 vs whisper base.en (previous engine),
  and cleanup latency Fast vs Smart. Script prints machine, OS, CPU, versions.
- **UI**: Vite build + Playwright smoke against the dev server (onboarding flow,
  empty/error states, reduced motion).
- **CI**: GitHub Actions matrix `macos-latest` + `windows-latest`: fmt, clippy
  `-D warnings`, tests, `tauri build`. Release workflow builds and uploads both
  installers + `latest.json`.
- **QA pass**: a QA agent works the matrix above on macOS for real, and on
  Windows through the CI-built installer (runner smoke: install silently, launch,
  sidecar health check, uninstall). Real-hardware Windows testing is called out as
  not yet done in the release notes until a human does it.

## README and landing page

- **README:** hero (logo + one line "Speak softly. Send it clean."), 30-second
  demo GIF, Install (macOS / Windows), Quick guide, Privacy, Benchmark (table +
  how to reproduce), Case study (VendastaTalk → Purl: what problem, what changed,
  measured results), Credits & licences (Parakeet CC-BY-4.0, transcribe-rs, ONNX Runtime, llama.cpp,
  Qwen, fonts), Contributing, MIT.
- **Landing page:** static site (Vite + plain TS, no framework needed), built from the 02-motion pack: serif hero with spectrum italic, the dictation pill listening, a live
  "messy speech → clean text" demo, three feature bands (private, fast, works
  everywhere), benchmark strip, download buttons that detect OS, FAQ, footer.
  Lighthouse ≥ 95 performance/accessibility, works at 360 px wide.

## Team and delivery

Agent team, each slice on its own branch + worktree in the new repo, each PR
reviewed by a fresh reviewer agent before merge:

| Role | Slice |
|---|---|
| Engine SWE | `engine/` (ModelStore, transcribe-rs STT, llama-server LLM), delete whisper/Ollama/BYOK |
| Platform SWE | `platform/` traits, Windows injector/locator/overlay, CI matrix, installers |
| Design lead | Shared `brand/` design system from the 02-motion pack |
| App designer + FE | App on the shared system: live pill, Home orb, all screens |
| Web designer + FE | `purl-site` landing page on the shared system |
| QA engineer | Edge-case matrix, benchmark runs, Playwright smoke |
| Reviewer | Fresh per PR |

## Risks

- **Parakeet on older Windows CPUs:** may run slower than real time. Windows uses Microsoft's
  official CPU-only ONNX Runtime 1.28.0 (no DirectML, no AVX2-only build) shipped as `onnxruntime.dll`
  beside `purl.exe`.
- **No Windows hardware in the loop:** CI smoke proves install + launch + engine
  health, not real paste into Chrome. Stated plainly in release notes.
- **Unsigned Windows installer:** SmartScreen warning; corp-managed machines may
  block it outright.
- **Ownership:** code originates from a Vendasta repo; the user has chosen to
  publish under MIT.
