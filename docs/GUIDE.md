# Purl guide

How to set up Purl and get the most out of it. To install it first, see the
[README](../README.md#install). If macOS or Windows blocks the first open,
see [First open](RELEASING.md#first-open-macos-and-windows).

## First run

Setup has four steps. You can leave and come back with **Settings → Run
setup again**.

1. **Welcome.**
2. **Permissions.**
   - **Microphone**, so Purl can hear you.
   - **Accessibility** (macOS only), so Purl can paste for you. It also lets
     Purl read the field it pasted into, to learn your fixes. See
     [Learn corrections](#learn-corrections-macos).
   - On Windows, if the microphone is blocked, turn it on in Windows
     **Settings → Privacy & security → Microphone**.
3. **Speech model.** Purl downloads Parakeet v3 (about 670 MB) once. After
   that, speech-to-text works offline.
4. **Try it.** Dictate into the box to check everything works.

After setup, Purl offers Smart cleanup. You can say no and turn it on later.

## Dictate

1. Click where you want the text: an email, a chat box, a document.
2. **Hold** the hotkey and talk:
   - macOS: **⌘⇧Space** (Command + Shift + Space)
   - Windows: **Ctrl+Shift+Space**
3. **Let go.** Purl turns your speech into text, cleans it up and pastes it
   at your cursor.

A small pill shows while you talk. A tap shorter than about a third of a
second is ignored, so a slip of the finger doesn't paste anything.

You can also start and stop dictation from the menu bar or tray icon
(**Start / Stop Dictation**), or with **Start dictation** on the Home screen.

The first dictation after Purl starts can take a few seconds while the
speech model loads. Purl unloads the model after 15 minutes of no use, to
free memory, and loads it again when you next talk.

**Change the hotkey:** Settings → Hotkey. If the new hotkey is already taken,
Purl says so and keeps the old one.

## Voice commands

Say a command **on its own** (as its own sentence, or after a comma) and
Purl acts on it instead of typing it. "The numbers go down" is typed as
words; "Go down." moves the cursor.

| Say | Does |
|---|---|
| "new line", "next line" | Starts a new line |
| "new paragraph" | Leaves a blank line |
| "press enter" | Presses Enter |
| "press tab" | Presses Tab |
| "go up" / "go down" / "go left" / "go right" | Moves the cursor |

"move …" and "cursor …" work the same as "go …". Turn voice commands off in
Settings → Voice commands.

## Cleanup

Settings → Cleanup has three modes.

| Mode | What it does | Download |
|---|---|---|
| **Off** | Pastes exactly what Purl heard. | None |
| **Fast** (default) | Removes "um", "uh" and similar, and fixes the casing of your dictionary words. Instant. | None |
| **Smart** | Also drops false starts and fixes misheard words and small grammar slips, using a small AI model on your computer. | Qwen3.5 2B, about 1.4 GB |

Smart has a larger option, Qwen3.5 4B (about 3 GB). It's slower.

Smart has a time limit: 6 seconds for the 2B model, 10 for the 4B. If it
runs out of time, or its answer looks wrong (empty, far too long or too
short), Purl pastes the Fast result instead. You always get your text.

Smart runs a helper program, `llama-server`, that starts when you begin
talking and stops after 5 minutes without use. The first start takes a while
(19.1 s on the benchmark machine; see the
[README benchmark](../README.md#benchmark)), so your first Smart dictation
may come back as Fast.

## Format

Settings → Format shapes the text for where it's going.

- **As spoken:** no change.
- **Email:** greeting on its own line, the body in short paragraphs, and the
  sign-off at the end.
- **Coding:** for prompts and technical notes. Start a sentence with
  "bullet point" or "next bullet" to make a list item. With Smart, it also
  keeps technical casing (camelCase, file names), puts code words in
  `backticks`, and opens or closes a code block when you say "code block".

## Paste or copy

Settings → **When text is ready**:

- **Paste at the cursor** (default).
- **Copy to the clipboard only.** You paste it yourself.

If the paste fails, Purl leaves the text on the clipboard and tells you. On
macOS that happens until you allow Accessibility.

## Dictionary

Add names, product words and jargon so Purl spells them your way. Use
**Written as** for short forms, like "pr" written as "PR".

The dictionary starts with one word, **Purl**, because the speech model
tends to hear it as "pearl".

- **Fast** fixes the casing of dictionary words.
- **Smart** also uses the list as a hint for words it might mishear.

## Snippets

A snippet swaps a phrase you say for longer text. Add a trigger like "my
email address" and the full address. Say the trigger while dictating and
Purl types the full text instead. Snippets are plain text matching; no AI.

## Transforms

Select text in any app and press a transform's hotkey. Purl rewrites the
selection in place, on your computer.

| Transform | Hotkey | Does |
|---|---|---|
| Polish | **Alt+1** (Option+1 on Mac) | Fixes grammar, spelling and punctuation. Keeps your words and tone. |
| Prompt Engineer | **Alt+2** (Option+2 on Mac) | Turns rough notes into a clear prompt for an AI assistant. |

You can edit them, change their hotkeys, or add your own. **Reset to
defaults** brings the built-ins back.

Transforms need Smart cleanup turned on. If a transform fails, your
selected text is left as it was.

## Learn corrections (macOS)

When you fix a misheard word by hand right after dictating, Purl notices
and spells it your way next time.

To do that, Purl reads the text field it pasted into for up to 90 seconds
after pasting (less once you stop editing). Only the fixed word and its
neighbours are kept, on this Mac. Password fields are never read.

It's on by default on macOS. Turn it off in Settings → Learn corrections.
**Dictionary → Learned from your fixes** lists what Purl has learned;
**Forget all** clears it.

Windows doesn't have this yet.

## Home: history and stats

Home shows your last dictation (raw and cleaned), your word counts and
words per minute, and your recent dictations. Purl keeps your last 50 dictations on your computer.
**Clear history** deletes them.

## Updates

Purl checks for updates a few seconds after it starts. When one is ready, it
asks you. Click **Update** to install and restart, or **Later** to be asked
again next time. You can also check from Settings → Updates.

## Where Purl keeps your data

Everything stays on your computer, in Purl's app-data folder:

- macOS: `~/Library/Application Support/app.purl.desktop/`
- Windows: `%APPDATA%\app.purl.desktop\`

That folder holds your settings, dictionary, snippets, transforms, learned
fixes, history, stats, a diagnostic log (`purl.log`) and the downloaded
models (in `models/`). Purl doesn't save your audio. Uninstalling the app
leaves this folder; delete it yourself to remove everything.

## Troubleshooting

| Problem | Try |
|---|---|
| Nothing is pasted (macOS) | Check System Settings → Privacy & Security → Accessibility has Purl turned on. Your text is on the clipboard meanwhile. |
| Purl can't hear you | Check the microphone permission (macOS System Settings, or Windows Settings → Privacy & security → Microphone). |
| "Still processing the previous dictation" | Wait a moment. Purl handles one dictation at a time. |
| Smart never seems to run | Check Settings → Cleanup is on Smart and the model finished downloading. The first Smart dictation after a pause may fall back to Fast while the model starts. |
| A model download failed | Click Retry. Downloads resume where they stopped. |

Still stuck? [Open an issue](https://github.com/Fan1000-prog/purl/issues).
The log in the data folder above helps; it records what failed, but check it
before you share it.
