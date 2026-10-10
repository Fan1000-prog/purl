import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  getSettings,
  saveOnboardingComplete,
  prettyHotkey,
  speechCheck,
  type TranscriptPayload,
} from "../main";
import type { EngineState, ModelView } from "../types";
import { createHotkeyRecorder, HOTKEY_RECORDER_CANCEL } from "./hotkey-recorder";

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

interface StatusPayload {
  stt: EngineState;
  accessibility_trusted: boolean;
  hotkey: string;
  state: string;
}

const TOTAL_STEPS = 5;
let step = 1;
let open = false;
let pollTimer: ReturnType<typeof setInterval> | undefined;
let gotFirstResult = false;

function el(tag: string, className: string, text: string): HTMLElement {
  const node = document.createElement(tag);
  if (className) node.className = className;
  node.textContent = text;
  return node;
}

function check(ok: boolean, label: string, fix: string): HTMLLIElement {
  const li = document.createElement("li");
  li.className = ok ? "ok" : "bad";
  li.appendChild(el("span", "mark", "●"));
  li.appendChild(document.createTextNode(` ${label}`));
  if (!ok && fix) li.appendChild(el("div", "fix", fix));
  return li;
}

function hotkeyChip(): HTMLElement {
  const kbd = document.createElement("kbd");
  kbd.textContent = prettyHotkey(getSettings()?.hotkey ?? "CmdOrCtrl+Shift+Space");
  return kbd;
}

function setChrome(title: string, nextLabel: string, nextEnabled: boolean) {
  $("wiz-title").textContent = title;
  $<HTMLSpanElement>("wiz-progress").textContent = `${step} of ${TOTAL_STEPS}`;
  const next = $<HTMLButtonElement>("wiz-next");
  next.textContent = nextLabel;
  next.disabled = !nextEnabled;
}

function renderWelcome(body: HTMLElement) {
  setChrome("Talk instead of type.", "Set me up", true);
  body.appendChild(
    el("p", "",
      "Hold a hotkey, say what you mean, release — and it's typed wherever your cursor is. Punctuation fixed, filler words gone, your words spelled right."),
  );
  body.appendChild(
    el("p", "",
      "Your voice never leaves this Mac. Transcription and cleanup both run here, and audio is never uploaded — not under any setting."),
  );
  body.appendChild(
    el("p", "",
      "One thing worth knowing up front. After each paste, Purl reads that text field for 30 seconds to catch words you fix by hand, so it can spell them right next time. Fields macOS marks as secure text — password fields — are never read, and it's a switch you control in Settings."),
  );
}

function renderHotkey(body: HTMLElement) {
  const current = prettyHotkey(getSettings()?.hotkey ?? "CmdOrCtrl+Shift+Space");
  setChrome("Your dictation key.", `Keep ${current}`, true);
  body.appendChild(
    el("p", "",
      "Hold this key combo anywhere to talk. Keep the default, or press Change and hit the keys you want."),
  );
  body.appendChild(
    createHotkeyRecorder({
      onSaved: () => {
        $<HTMLButtonElement>("wiz-next").textContent = "Continue";
      },
    }),
  );
}

async function renderChecks(body: HTMLElement) {
  setChrome("Quick check.", "Continue", false);
  body.appendChild(
    el("p", "", "Making sure everything's in place. Red items come with a fix."),
  );
  const ul = document.createElement("ul");
  ul.className = "wiz-checks";
  body.appendChild(ul);
  const note = el("p", "", "");
  body.appendChild(note);

  const update = async () => {
    if (!open || step !== 3) {
      stopPolling();
      return;
    }
    try {
      const s = await invoke<StatusPayload>("get_status");
      const items = [check(...(await speechRow(s.stt)))];
      items.push(
        check(s.accessibility_trusted,
          "Accessibility permission — lets it type for you, and read the field it typed into",
          "System Settings → Privacy & Security → Accessibility → turn on Purl. You can do this later — transcripts will go to your clipboard instead. This one grant covers both: pasting at your cursor, and reading back the field afterwards to catch words you fix by hand. macOS asks once, for both. Turn the reading half off any time in Settings → Learn corrections."),
      );
      ul.replaceChildren(...items);
      note.textContent = "";
      // The speech model gates Continue; Accessibility is deferrable.
      $<HTMLButtonElement>("wiz-next").disabled =
        !["ready", "idle", "loading"].includes(s.stt.state);
    } catch {
      note.textContent = "Can't check right now — you can continue and come back to Settings → Setup later.";
      $<HTMLButtonElement>("wiz-next").disabled = false;
    }
  };
  await update();
  if (open && step === 3) {
    pollTimer = setInterval(() => void update(), 2000);
  }
}

function renderFirstDictation(body: HTMLElement) {
  setChrome("Try it.", "Nice — what else can it do?", gotFirstResult);
  const p = el("p", "", "Click into any text field, hold ");
  p.appendChild(hotkeyChip());
  p.appendChild(
    document.createTextNode(
      " and say: “Testing Purl, one two three.” Then release.",
    ),
  );
  body.appendChild(p);
  const result = el("p", "transcript hidden", "");
  result.id = "wiz-result";
  body.appendChild(result);
  const cheer = el("p", "hidden", "That's it. It typed itself.");
  cheer.id = "wiz-cheer";
  body.appendChild(cheer);
  const err = el("p", "hidden", "");
  err.id = "wiz-error";
  body.appendChild(err);
}

function renderFeatures(body: HTMLElement) {
  setChrome("It does more than type.", "Start talking", true);
  body.appendChild(
    el("p", "", "Four things worth knowing about — all one click from Home."),
  );
  // Clone the home feature grid; make cards inert (tour, not nav).
  const grid = document
    .querySelector<HTMLDivElement>("#view-home .feature-grid")
    ?.cloneNode(true) as HTMLDivElement | null;
  if (grid) {
    grid.querySelectorAll("button").forEach((b) => (b.disabled = true));
    body.appendChild(grid);
  }
}

function stopPolling() {
  if (pollTimer !== undefined) {
    clearInterval(pollTimer);
    pollTimer = undefined;
  }
}

function render() {
  stopPolling();
  const body = $("wiz-body");
  document.dispatchEvent(new Event(HOTKEY_RECORDER_CANCEL));
  body.replaceChildren();
  if (step === 1) renderWelcome(body);
  else if (step === 2) renderHotkey(body);
  else if (step === 3) void renderChecks(body);
  else if (step === 4) renderFirstDictation(body);
  else renderFeatures(body);
}

function close() {
  document.dispatchEvent(new Event(HOTKEY_RECORDER_CANCEL));
  stopPolling();
  open = false;
  $("onboarding").classList.add("hidden");
  void saveOnboardingComplete().catch(() => {
    // Non-fatal: wizard reappears next launch, which is acceptable.
  });
}

export function openOnboarding() {
  step = 1;
  gotFirstResult = false;
  open = true;
  $("onboarding").classList.remove("hidden");
  render();
}

export function initOnboarding() {
  $<HTMLButtonElement>("wiz-skip").addEventListener("click", close);
  $<HTMLButtonElement>("wiz-next").addEventListener("click", () => {
    if (step >= TOTAL_STEPS) {
      close();
      return;
    }
    step += 1;
    render();
  });
  $<HTMLButtonElement>("replay-tour").addEventListener("click", openOnboarding);

  void listen<TranscriptPayload>("result", (e) => {
    if (!open || step !== 4) return;
    gotFirstResult = true;
    const result = $("wiz-result");
    result.textContent = e.payload.cleaned;
    result.classList.remove("hidden");
    $("wiz-cheer").classList.remove("hidden");
    $("wiz-error").classList.add("hidden");
    $<HTMLButtonElement>("wiz-next").disabled = false;
  });
  void listen<string>("pipeline-error", (e) => {
    if (!open || step !== 4) return;
    const err = $("wiz-error");
    err.textContent = `Hmm, that didn't work (${e.payload}). Check the previous step's red items, or skip and try from the app.`;
    err.classList.remove("hidden");
  });

  if (getSettings()?.onboarding_complete === false) openOnboarding();
}

const SPEECH_MODEL = "parakeet-v3";

/**
 * The speech-model checklist row. While it isn't downloaded, starts the
 * download (a no-op if one is already running) and shows its progress.
 */
async function speechRow(stt: EngineState): Promise<[boolean, string, string]> {
  if (stt.state !== "missing") return speechCheck(stt);
  const model = (await invoke<ModelView[]>("models_status")).find((m) => m.id === SPEECH_MODEL);
  if (model?.state === "failed") {
    return [false, "Speech model download failed", `${model.error ?? ""} Retrying…`];
  }
  if (model?.state !== "downloading") await invoke("download_model", { id: SPEECH_MODEL });
  const pct = model && model.size_bytes > 0
    ? Math.floor((model.downloaded_bytes / model.size_bytes) * 100)
    : 0;
  return [false, `Downloading the speech model… ${pct}%`, "About 670 MB, one time. Keep this window open."];
}
