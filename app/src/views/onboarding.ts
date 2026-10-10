// First-run setup, four steps: welcome → permissions → speech model download
// → try it. Smart cleanup is offered afterwards on Home (smart-offer.ts), so it
// never sits in the critical path. "Set up later" is always there: a user who
// is offline or busy can finish setup from Home and Settings.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getSettings, saveOnboardingComplete, type TranscriptPayload } from "../main";
import { mountMascot, type Mascot } from "../components/mascot";
import { mountModelDownload, type ModelDownload } from "../components/model-download";
import { displayHotkey, platform } from "../platform";
import type { EngineState, EngineStatusView, Trust } from "../types";
import { showSmartOffer } from "./smart-offer";

const STEPS = ["welcome", "permissions", "model", "try"] as const;
type Step = (typeof STEPS)[number];
const SPEECH_MODEL = "parakeet-v3";
const PERMISSION_POLL_MS = 2000;

const SETTINGS_URLS = {
  macos: {
    mic: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
    ax: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
  },
  windows: { mic: "ms-settings:privacy-microphone", ax: "" },
};

interface StatusPayload {
  accessibility_trusted: boolean;
}

let root: HTMLElement;
let step: Step = "welcome";
let mascot: Mascot | null = null;
let download: ModelDownload | null = null;
let pollTimer: ReturnType<typeof setInterval> | undefined;
let sttState: EngineState["state"] = "missing";

function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> & { className?: string } = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function button(label: string, kind: "primary" | "ghost", onClick: () => void): HTMLButtonElement {
  const b = h("button", { type: "button", className: kind === "primary" ? "btn btn-primary" : "btn" }, label);
  b.addEventListener("click", onClick);
  return b;
}

function hotkey(): string {
  return displayHotkey(getSettings()?.hotkey ?? "CmdOrCtrl+Shift+Space");
}

// ---------- chrome ----------

function teardownStep() {
  clearInterval(pollTimer);
  pollTimer = undefined;
  mascot?.destroy();
  mascot = null;
  download?.destroy();
  download = null;
}

function frame(title: string, body: Node[], actions: Node[], above: Node[] = []) {
  const index = STEPS.indexOf(step);
  const dots = h("ol", { className: "onb-dots" });
  dots.setAttribute("aria-label", `Step ${index + 1} of ${STEPS.length}`);
  STEPS.forEach((_, i) => dots.append(h("li", { className: i <= index ? "on" : "" })));

  const later = step === "welcome" ? [] : [h("button", { type: "button", className: "onb-later" }, "Set up later")];
  later[0]?.addEventListener("click", () => void finish());

  root.replaceChildren(
    h(
      "div",
      { className: "onb-panel" },
      dots,
      ...above,
      h("h1", { id: "onb-title", className: "onb-title" }, title),
      ...body,
      h("div", { className: "onb-actions" }, ...actions),
      ...later,
    ),
  );
  root.querySelector<HTMLElement>(".btn-primary:not(:disabled), textarea")?.focus();
}

function go(next: Step) {
  teardownStep();
  step = next;
  root.dataset.step = next;
  ({ welcome, permissions, model, try: tryIt })[next]();
}

const back = () => go(STEPS[STEPS.indexOf(step) - 1]);

// ---------- 1. welcome ----------

function welcome() {
  const art = h("div", { className: "onb-mascot" });
  frame(
    "Speak softly. Send it clean.",
    [
      h(
        "p",
        { className: "onb-lede" },
        `Press ${hotkey()}, talk, press it again. Purl types your words where your cursor is, tidied up. It all runs on this computer: no account, and your voice never leaves it.`,
      ),
    ],
    [button("Set up Purl", "primary", () => go("permissions"))],
    [art],
  );
  mascot = mountMascot(art, {
    size: 120,
    surface: matchMedia("(prefers-color-scheme: dark)").matches ? "deep" : "cream",
  });
}

// ---------- 2. permissions ----------

const TRUST_LABEL: Record<Trust | "unknown", string> = {
  granted: "Allowed",
  denied: "Not allowed",
  not_required: "Not needed",
  unknown: "Asked the first time you dictate",
};

function permissionRow(name: string, why: string, url: string) {
  const status = h("span", { className: "perm-status" });
  const row = h(
    "li",
    { className: "perm-row" },
    h("div", {}, h("div", { className: "perm-name" }, name), h("div", { className: "perm-why muted" }, why)),
    status,
    button("Open Settings", "ghost", () => void openUrl(url)),
  );
  const set = (t: Trust | "unknown") => {
    row.dataset.trust = t;
    status.textContent = TRUST_LABEL[t];
  };
  return { row, set };
}

async function micTrust(): Promise<Trust | "unknown"> {
  // mic_status lands with the platform layer; until then the OS asks on the
  // first recording, which is what we say.
  return invoke<Trust>("mic_status").catch(() => "unknown" as const);
}

function permissions() {
  const os = platform();
  const mic = permissionRow("Microphone", "So Purl can hear you.", SETTINGS_URLS[os].mic);
  const rows = [mic.row];
  const ax = os === "macos"
    ? permissionRow(
        "Accessibility",
        "So Purl can paste at your cursor. Without it, your words go to the clipboard. " +
          "It also lets Purl read that text field for 30 seconds after pasting, to learn words you fix by hand. " +
          "Password fields are never read, and you can turn this off in Settings.",
        SETTINGS_URLS.macos.ax,
      )
    : null;
  if (ax) rows.push(ax.row);

  const note = h("p", { className: "onb-note warn-text", hidden: true });
  frame(
    "Two quick permissions.",
    [
      h("p", { className: "onb-lede" }, os === "macos"
        ? "macOS asks before any app can hear you or type for you. Turn Purl on in each, then come back here."
        : "Windows asks before any app can use your microphone. Turn it on, then come back here."),
      h("ul", { className: "perm-list" }, ...rows),
      note,
    ],
    [button("Back", "ghost", back), button("Continue", "primary", () => go("model"))],
  );

  const update = async () => {
    const m = await micTrust();
    mic.set(m);
    let axTrust: Trust = "not_required";
    if (ax) {
      const s = await invoke<StatusPayload>("get_status").catch(() => null);
      axTrust = s ? (s.accessibility_trusted ? "granted" : "denied") : "denied";
      ax.set(axTrust);
    }
    note.hidden = m !== "denied";
    note.textContent = "Purl can't hear you without the microphone. You can still continue and turn it on later.";
  };
  void update();
  pollTimer = setInterval(() => void update(), PERMISSION_POLL_MS);
}

// ---------- 3. speech model ----------

function model() {
  const slot = h("div");
  const next = button("Continue", "primary", () => go("try"));
  next.disabled = true;
  frame(
    "Download the speech model.",
    [
      h("p", { className: "onb-lede" }, "Purl turns speech into text on your computer, so it needs this model once. After that it works offline."),
      slot,
    ],
    [button("Back", "ghost", back), next],
  );
  download = mountModelDownload(slot, {
    id: SPEECH_MODEL,
    title: "Parakeet v3",
    detail: "runs on your computer",
    onReady: () => {
      next.disabled = false;
      next.focus();
    },
  });
}

// ---------- 4. try it ----------

function tryIt() {
  const box = h("textarea", { className: "onb-try", rows: 4, placeholder: "Your words will appear here." });
  box.setAttribute("aria-label", "Try dictating here");
  const status = h("p", { className: "onb-note muted", hidden: true });
  status.setAttribute("role", "status");
  frame(
    "Try it.",
    [h("p", { className: "onb-lede" }, `Press ${hotkey()} and say something, then press it again.`), box, status],
    [button("Back", "ghost", back), button("Finish", "primary", () => void finish())],
  );
  showWarmup();
}

function showWarmup() {
  const status = root.querySelector<HTMLElement>(".onb-note");
  if (!status || step !== "try") return;
  status.hidden = sttState !== "loading";
  status.textContent = "Warming up the speech model. Your first dictation may take a few seconds.";
}

// ---------- open / close ----------

async function finish() {
  teardownStep();
  root.classList.add("hidden");
  await saveOnboardingComplete().catch(() => {
    // Non-fatal: setup shows again next launch.
  });
  void showSmartOffer();
}

export function openOnboarding() {
  root.classList.remove("hidden");
  go("welcome");
}

export function initOnboarding() {
  root = document.getElementById("onboarding")!;
  document.getElementById("replay-tour")?.addEventListener("click", openOnboarding);

  void listen<TranscriptPayload>("result", (e) => {
    if (step !== "try" || root.classList.contains("hidden")) return;
    const box = root.querySelector<HTMLTextAreaElement>(".onb-try");
    if (box) box.value = e.payload.cleaned;
  });

  const onEngine = (v: EngineStatusView) => {
    sttState = v.stt.state;
    showWarmup();
  };
  void listen<EngineStatusView>("engine-status", (e) => onEngine(e.payload));
  // engine_status arrives with the speech engine; until then, no warm-up note.
  void invoke<EngineStatusView>("engine_status").then(onEngine, () => {});

  if (getSettings()?.onboarding_complete === false) openOnboarding();
  else void showSmartOffer();
}
