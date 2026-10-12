<p align="center">
  <img src="app/src/brand/logo/purl-app-icon-macos-deep.svg" width="96" height="96" alt="Purl app icon">
</p>

<h1 align="center">Purl</h1>

<p align="center"><em>Speak softly. Send it clean.</em></p>

<p align="center">
  <a href="https://github.com/Fan1000-prog/purl/actions/workflows/ci.yml"><img src="https://github.com/Fan1000-prog/purl/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-MIT-3F6B4B" alt="MIT licence"></a>
  <a href="https://github.com/Fan1000-prog/purl/releases/latest"><img src="https://img.shields.io/github/v/release/Fan1000-prog/purl" alt="Latest release"></a>
</p>

Purl is a free, open-source dictation app for macOS and Windows. Hold a
hotkey, talk, and let go. Purl turns your speech into text on your own
computer, tidies it up, and pastes it where your cursor is: Gmail, Google
Chat, VS Code, or any text box. No account, no API keys, and your audio
never leaves your computer.

## Install

Purl runs on **macOS with Apple Silicon** (M1 or newer) and **Windows 10/11
x64**. Intel Macs aren't supported yet.

The installers are not signed by Apple or Microsoft yet, so both systems
warn you the first time you open Purl. The steps below get you past that.

### macOS (Apple Silicon)

1. Download [`Purl_aarch64.dmg`](https://github.com/Fan1000-prog/purl/releases/latest/download/Purl_aarch64.dmg).
2. Open it and drag **Purl** to **Applications**.
3. Open Purl the first time:
   - **macOS 14:** in Applications, right-click Purl → **Open**, then
     **Open** again.
   - **macOS 15 and later:** right-click → Open no longer works. Open Purl
     once and close the warning. Then go to **System Settings → Privacy &
     Security**, scroll down to the message about Purl, click **Open
     Anyway** and confirm with your password.
4. Follow the setup. It asks for **Microphone** (to hear you) and
   **Accessibility** (to paste for you), then downloads the speech model
   (about 670 MB, once).

### Windows (x64)

1. Download [`Purl_x64-setup.exe`](https://github.com/Fan1000-prog/purl/releases/latest/download/Purl_x64-setup.exe).
2. Run it. On the blue "Windows protected your PC" screen, click **More
   info → Run anyway**. It installs for your user only; no admin needed.
3. Follow the setup. It downloads the speech model (about 670 MB, once).

> **Windows is untested on real hardware.** Purl builds, installs, starts
> and uninstalls on Windows in CI, but nobody has dictated with it on a real
> Windows PC yet. If you try it, please
> [open an issue](https://github.com/Fan1000-prog/purl/issues) and say how
> it went.

After the first open, both systems remember your choice. Updates install
from inside the app.

## Quick guide

| Do this | To |
|---|---|
| Hold **⌘⇧Space** (Mac) or **Ctrl+Shift+Space** (Windows), talk, let go | Dictate. The text is pasted at your cursor. |
| Say "new line", "new paragraph" or "press enter" on its own | Move the cursor instead of typing the words |
| Pick **Off**, **Fast** or **Smart** in Settings → Cleanup | Choose how much tidying happens |
| Add names to **Dictionary** | Spell names and jargon your way |
| Add a **Snippet** like "my email address" | Say the trigger, get the full text |
| Select text, press **Alt+1** (Polish) or **Alt+2** (Prompt Engineer) | Rewrite it with a **Transform** (needs Smart) |

Cleanup modes:

- **Off** pastes exactly what was heard.
- **Fast** (the default) removes "um" and "uh" and fixes the casing of your
  dictionary words. It takes no time and needs no extra download.
- **Smart** (opt-in) also drops false starts and fixes misheard words and
  small grammar slips, with a small AI model that runs on your computer
  (Qwen3.5 2B, about 1.4 GB download). If it's slow or gives something odd
  back, Purl pastes the Fast result instead.

The full guide is in [docs/GUIDE.md](docs/GUIDE.md).

## Privacy

Speech-to-text and cleanup run on your computer. Purl goes online for only
two things:

1. **Model downloads**, from Hugging Face, when you ask for them in setup or
   Settings.
2. **The update check**, against this repo's GitHub Releases, a few seconds
   after Purl starts and when you click **Check for updates**.

Nothing is sent while you dictate. Smart cleanup runs as a helper program
that only listens on your own computer (`127.0.0.1`).

On macOS, **Learn corrections** (on by default) reads the text field Purl
pasted into for up to 90 seconds after pasting (less once you stop
editing), to learn words you fix by hand. Password fields are never read,
and you can turn this off in Settings. Windows doesn't have this yet.

Details: [docs/PRIVACY.md](docs/PRIVACY.md).

## Benchmark

Measured on an **Apple M1 Pro** (10 cores, 16 GB, macOS 26.4.1) on
2026-10-12, Purl commit `4627208`. The machine was busy during the run
(load average 18 to 26), so the speed numbers are on the slow side. Windows:
not measured yet.

**Accuracy** (word error rate, lower is better):

| Test set | Parakeet v3 (Purl) | whisper.cpp base.en (old engine) |
|---|---|---|
| LibriSpeech test-clean, 100 clips, 15 min | 3.2% <!-- update after the TDT fix --> | 2.9% |
| 10 dictation clips, synthetic (read by macOS `say`) | 0.8% | 1.6% |

The dictation clips are synthetic: a computer voice read them, so they're
cleaner than real speech. Treat them as a check that everyday office words
come out right, not as a real-world number.

**Speed** (real-time factor: processing time ÷ audio length, lower is
better; 0.04 means a 10-second clip takes about 0.4 seconds):

| Engine | LibriSpeech, mean | LibriSpeech, p95 | Model load (cold) |
|---|---|---|---|
| Parakeet v3, int8, CPU (Purl) | 0.041 | 0.052 | 1101 ms |
| whisper.cpp base.en, Metal GPU | 0.026 | 0.046 | 81 ms |
| whisper.cpp base.en, CPU only | 0.350 | 0.659 | 68 ms |

**Cleanup time** (on the 10 dictation transcripts):

| Mode | Mean | p95 | Notes |
|---|---|---|---|
| Fast | 0.0031 ms | 0.0037 ms | 10 calls |
| Smart (Qwen3.5 2B) | 910 ms | 983 ms | 30 calls after one warm-up, 0 fell back to Fast; first server start 19.1 s |

Raw results: `bench/results/2026-10-12-apple-m1-pro.json`. To run it
yourself, see `bench/README.md`.

## Case study

Purl grew out of an in-house dictation tool for office staff: Mac only,
installed from Terminal with Homebrew and a separate AI app. Purl keeps what
worked, drops the setup, adds Windows and a newer speech model. Read the
whole story in [docs/CASE-STUDY.md](docs/CASE-STUDY.md).

## Credits and licences

| What | By | Licence |
|---|---|---|
| Parakeet v3 speech model (`parakeet-tdt-0.6b-v3`) | NVIDIA | CC-BY-4.0 |
| transcribe-rs | | MIT |
| ONNX Runtime | Microsoft | MIT |
| llama.cpp (`llama-server`) | | MIT |
| Qwen3.5 (Smart cleanup) | Alibaba Cloud | Apache-2.0 |
| Instrument Serif, Geist, Geist Mono fonts | | SIL Open Font License 1.1 |
| LibriSpeech (benchmark only) | | CC-BY-4.0 |
| whisper.cpp (benchmark baseline only, not in the app) | | MIT |

Inspired by [Handy](https://github.com/cjpais/Handy) and
[OpenWhispr](https://github.com/OpenWhispr/openwhispr).

## Contributing

Issues and pull requests are welcome. Build from source:

```bash
cd app && npm ci && npm run tauri build
```

Tests, lint and project conventions are in [AGENTS.md](AGENTS.md).
How releases are cut and signed: [docs/RELEASING.md](docs/RELEASING.md).

## Licence

MIT. See [LICENSE](LICENSE).
