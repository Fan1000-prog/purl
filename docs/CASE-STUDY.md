# Case study: from VendastaTalk to Purl

Purl started life as **VendastaTalk**, a dictation app built for staff at
Vendasta. This is how it got from there to a free, open-source app anyone
can install.

## The problem

Staff write a lot: emails in Gmail, messages in Google Chat, notes, tickets.
Talking is faster than typing, but dictation at work has two problems:

- **Jargon.** Product names and company terms come out wrong.
- **Privacy.** Many dictation tools send your audio to a cloud service. For
  work conversations, that's a hard sell.

The goal was simple: hold a key, talk, and get clean text where the cursor
is, with the audio never leaving the laptop.

## VendastaTalk (July to August 2026)

The first version landed on 2026-07-03. It was a Tauri app for macOS with
two local engines:

- **whisper.cpp** for speech-to-text, installed through Homebrew.
- **Ollama** running a small local AI model for cleanup.

Over the next six weeks it grew, one release at a time:

| Date | What changed |
|---|---|
| 2026-07-03 | First version: hold-to-talk, local transcription, one-command installer |
| 2026-07-05 | The speech model stays loaded, instead of loading on every dictation |
| 2026-07-14 | Voice commands and writing modes (Email, Coding) |
| 2026-07-16 | Transforms: rewrite selected text with a hotkey |
| 2026-07-17 | v0.4.0, the first release that updates itself |
| 2026-07-18 | v0.5.0, one build for Intel and Apple Silicon Macs |
| 2026-07-30 | Optional cloud cleanup with your own API key |
| 2026-08-02 | v0.7.0: learning from your corrections |
| 2026-08-14 | v0.8.0, the last VendastaTalk release |

### The hard lesson: correction learning

Correction learning means: fix a misheard word by hand, and the app spells
it your way next time. To see the
fix, the app reads the text field it just pasted into, using the macOS
Accessibility API.

It shipped in v0.7.0. It didn't work. Neither did the fix in v0.7.1. Both
releases said it worked.

The cause was a quiet detail of macOS. Asking the system-wide Accessibility
element for "the focused field" often failed with no answer. Asking the
**frontmost app's** element worked straight away. Measured side by side, on
the same field at the same moment: the system-wide route worked 0 times out
of 3, the app route 3 times out of 3. v0.7.2 switched routes, and learning
finally worked.

Two wrong guesses had shipped before that. What ended it was not a third
guess but a small probe that printed what the API really returned. The
lessons stuck:

- **Measure before you guess**, especially where tests use mocks. All the
  unit tests passed through both broken releases, because the part that was
  broken was the part the tests replaced with a fake.
- **A fix in the build is not a working fix.** Check the behaviour, not
  just that the code shipped.
- **Say what isn't verified.** A release note that admits a gap costs
  nothing. One that claims too much has to be taken back in public.

Purl carries these forward. Its docs say plainly that Windows hasn't been
tested on real hardware, and the benchmark marks its synthetic clips as
synthetic.

## Why Purl

By v0.8.0, VendastaTalk did the job on a Mac. But it had limits that came
from how it was built:

- **Mac only**, and installed from Terminal: Homebrew, whisper.cpp, Ollama
  and a 2 GB AI model, before the app itself.
- **API keys.** The cloud cleanup option meant keys to store and explain.
- **An older speech model.** whisper.cpp base.en is English only.

Purl is the rebuild, started on 2026-10-09:

| | VendastaTalk v0.8.0 | Purl v1 |
|---|---|---|
| Platforms | macOS (Intel and Apple Silicon) | macOS Apple Silicon and Windows x64 |
| Install | Terminal script: Homebrew, whisper.cpp, Ollama, models, app | Download an installer, open it; the speech model downloads inside the app |
| Speech-to-text | whisper.cpp base.en, a separate server process | Parakeet v3, inside the app (transcribe-rs on ONNX Runtime) |
| Cleanup | Fast rules, or Ollama, or a cloud model with your key | Off, Fast, or Smart (Qwen3.5 on a bundled `llama-server`) |
| Network | Ollama on localhost, update check, cloud provider if you opted in | Model downloads and the update check, nothing else |
| Field reading for learning | up to 90 seconds (the app said 30) | up to 90 seconds after pasting (less once you stop editing) |
| Licence | Internal | MIT |

What changed under the hood:

- **No keys, no accounts.** The cloud cleanup and its key storage were
  removed, not hidden.
- **No Homebrew, no Ollama.** The speech model runs inside the app. Smart
  cleanup ships its own `llama-server`, which only listens on `127.0.0.1`.
- **Windows.** A platform layer handles pasting, permissions and helper
  processes on both systems. Correction learning stays macOS-only for now.
- **Fresh history.** VendastaTalk's git history held internal context, so
  Purl's repo starts from one import commit. This page tells the backstory
  instead.

Intel Macs were dropped. Parakeet needs a newer ONNX Runtime than Microsoft
builds for Intel Macs, so Intel support waits until people ask for it.

## Measured: old engine vs new

From the benchmark run on 2026-10-12, on an Apple M1 Pro
([`bench/results/2026-10-12-apple-m1-pro.json`](../bench/results/2026-10-12-apple-m1-pro.json)). The machine was busy during
the run, so speeds are on the slow side.

| | whisper.cpp base.en (VendastaTalk) | Parakeet v3 (Purl) |
|---|---|---|
| Word error rate, LibriSpeech (100 clips) | 2.9% | 3.2% <!-- update after the TDT fix --> |
| Word error rate, 10 dictation clips (synthetic) | 1.6% | 0.8% |
| Real-time factor, LibriSpeech, mean | 0.026 on the GPU, 0.350 on the CPU | 0.041 on the CPU |
| Model load, cold | 81 ms (GPU), 68 ms (CPU) | 1101 ms |

The dictation clips are synthetic: macOS `say` read them, so they're easier
than real speech.

Honest reading: on audiobook speech the old engine is a little more
accurate today, and with a GPU it's faster. Parakeet is ahead on the
dictation clips, and the model supports 25 European languages, where base.en
is English only. Purl has only been measured in English so far.
A fix to Purl's Parakeet decoder is being measured now; the LibriSpeech line
will be updated when it lands.

Cleanup on the same dictation transcripts:

| | Mean | p95 |
|---|---|---|
| Fast (rules) | 0.0031 ms | 0.0037 ms |
| Smart (Qwen3.5 2B, warm) | 910 ms | 983 ms |

Smart fell back to Fast 0 times in 30 calls. The first `llama-server` start
took 19.1 s. VendastaTalk's Ollama cleanup was never benchmarked, so there's
no before number for it: not measured.

Windows: not measured yet.

## What's next

- **Test on real Windows PCs.** So far Windows is covered by CI only.
- **Sign the installers**, so macOS and Windows stop warning on first open.
- **Re-measure Parakeet** after the decoder fix.
- **Correction learning on Windows.**
- **Intel Macs**, if people ask.
