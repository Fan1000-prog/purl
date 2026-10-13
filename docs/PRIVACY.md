# Privacy

Purl is built so your voice and your words stay on your computer. This page
says exactly what Purl does with them, and the only times it goes online.

## The short version

- Your audio never leaves your computer, and Purl doesn't save it.
- Speech-to-text and cleanup (Fast and Smart) run on your computer.
- Purl goes online for two things only: **model downloads** and **the update
  check**. Nothing is sent while you dictate.
- No account, no API keys, no analytics, no tracking.

## What goes online

### 1. Model downloads

Purl downloads models from Hugging Face, only when you ask:

| Model | When | Size |
|---|---|---|
| Parakeet v3 (speech) | During setup, or Settings → Models | about 670 MB |
| Qwen3.5 2B (Smart cleanup) | When you turn on Smart | about 1.4 GB |
| Qwen3.5 4B (Smart, larger) | Only if you pick it | about 3 GB |

Each file is checked against a fixed fingerprint (SHA-256) built into the
app. If it doesn't match, Purl deletes it.

### 2. The update check

Purl asks this project's GitHub Releases if there's a newer version. It
does this a few seconds after it starts, and when you click **Check for
updates**. If you choose to update, the new version downloads from the same
place, and Purl checks its signature before installing it.

Like any web request, these are seen by the host (Hugging Face or GitHub),
along with your IP address. Purl sends no account, ID or usage data with
them.

## What stays on your computer

### Your voice

Purl records only while you hold the hotkey (or after you click Start
dictation). The audio stays in memory, is turned into text on your
computer, and is then thrown away. It is never written to disk and never
sent anywhere.

### Smart cleanup

Smart cleanup runs a helper program, `llama-server`, on your computer. Purl
talks to it at `127.0.0.1`, an address only your own computer can reach, and
each run uses a new random key so other programs can't use it. Your text
goes to this helper and nowhere else.

### Your data

Purl keeps these in its app-data folder on your computer:

- settings, dictionary, snippets and transforms
- your last 50 dictations (the raw and cleaned text) and word counts
- learned fixes (macOS)
- a diagnostic log, `purl.log`
- the downloaded models

The folder is `~/Library/Application Support/app.purl.desktop/` on macOS and
`%APPDATA%\app.purl.desktop\` on Windows. **Clear history** (Home) and
**Forget all** (Dictionary) delete history and learned fixes. Delete the
folder to remove everything.

## What Purl reads from other apps

### Learn corrections (macOS only)

When you fix a misheard word right after dictating, Purl can learn it. In
Purl's own words:

> After pasting, Purl reads that text field for up to 90 seconds (less once
> you stop editing) to notice words you fix by hand. Only the fixed word and
> its neighbours are kept, on this Mac. Password fields are never read.

It's **on by default** on macOS, because it works through the Accessibility
permission Purl already needs to paste. macOS doesn't ask you about the
reading part separately, so setup tells you instead. Turn it off any time in
**Settings → Learn corrections**. It's also off when output is set to
**Copy to the clipboard only**.

Windows doesn't have this feature, so Purl never reads other apps' fields
there.

### Transforms

When you press a transform's hotkey, Purl copies your selected text,
rewrites it with Smart cleanup on your computer, and pastes the result.

### The clipboard

To paste, Purl puts the text on the clipboard, presses paste for you, then
puts back what was on your clipboard before. If the paste fails, it leaves
your text on the clipboard so you don't lose it.

## Text from other apps is treated as data

Text Purl reads from other apps (a selection, or a field it learns from) can
reach the Smart model. Purl wraps it in markers the text can't fake, and
tells the model it's data to clean, never instructions to follow.

## Questions

[Open an issue](https://github.com/Fan1000-prog/purl/issues). Purl is open
source, so you can also check all of this in the code.
