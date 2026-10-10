import { invoke } from "@tauri-apps/api/core";
import { getSettings, setCurrentHotkey, prettyHotkey } from "../main";
import { celebrate } from "../celebrate";

export const DEFAULT_HOTKEY = "CmdOrCtrl+Shift+Space";

/** Containers dispatch this on `document` before unmounting or hiding a
 *  recorder, so an armed capture can't outlive its UI. */
export const HOTKEY_RECORDER_CANCEL = "hotkey-recorder-cancel";

/** Keys that never complete a combo on their own. */
const MODIFIER_CODES = new Set([
  "MetaLeft", "MetaRight", "ControlLeft", "ControlRight",
  "AltLeft", "AltRight", "ShiftLeft", "ShiftRight", "CapsLock",
]);

/** KeyboardEvent.code (layout-independent) → Tauri accelerator key name. */
function codeToKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9])$/.test(code)) return code;
  const map: Record<string, string> = {
    Space: "Space", Enter: "Enter", Tab: "Tab", Backspace: "Backspace",
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
    Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown",
    Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]",
    Backslash: "\\", Semicolon: ";", Quote: "'", Comma: ",",
    Period: ".", Slash: "/", Backquote: "`",
  };
  return map[code] ?? null;
}

/**
 * Build a Tauri accelerator from a keydown event.
 * - null: not a complete combo yet (bare modifier / unsupported key)
 * - "needs-modifier": a main key was pressed with no modifier held
 * - otherwise: the accelerator string, e.g. "CmdOrCtrl+Shift+Space"
 */
export function eventToAccelerator(
  e: KeyboardEvent,
): string | "needs-modifier" | null {
  if (MODIFIER_CODES.has(e.code)) return null;
  const key = codeToKey(e.code);
  if (key === null) return null;
  const mods: string[] = [];
  if (e.metaKey) mods.push("CmdOrCtrl");
  if (e.ctrlKey && !e.metaKey) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (mods.length === 0) return "needs-modifier";
  return [...mods, key].join("+");
}

export interface HotkeyRecorderOpts {
  onSaved?: (accelerator: string | null) => void;
  /** Current combo (null = unassigned). Default: the dictation hotkey. */
  get?: () => string | null;
  /** Persist a combo; null clears it. Default: invoke("set_hotkey"). */
  set?: (accelerator: string | null) => Promise<void>;
  /** Combo the reset link restores; null hides the link.
   *  Default: DEFAULT_HOTKEY. */
  defaultHotkey?: string | null;
  /** Show a "Remove" link that clears the combo (transforms only). */
  allowClear?: boolean;
  /** Fire the confetti/toast celebration on a successful set. Default true;
   *  turned off for a new, not-yet-saved transform, since "Add transform"
   *  already celebrates that once the whole thing is actually saved. */
  celebrateOnSave?: boolean;
}

/**
 * Press-to-set hotkey widget: chip with the current combo, a Change button
 * that arms capture, and a reset-to-default link. Used in Settings,
 * onboarding, and the Transforms view (via the get/set pair). While armed,
 * every app-owned global shortcut is suspended so pressing one doesn't fire
 * its action.
 */
export function createHotkeyRecorder(opts?: HotkeyRecorderOpts): HTMLElement {
  const getCurrent =
    opts?.get ?? (() => getSettings()?.hotkey ?? DEFAULT_HOTKEY);
  const persist =
    opts?.set ??
    (async (accelerator: string | null) => {
      if (accelerator === null) {
        throw new Error("The dictation hotkey can't be removed.");
      }
      await invoke("set_hotkey", { newHotkey: accelerator });
      setCurrentHotkey(accelerator);
    });
  const defaultHotkey =
    opts?.defaultHotkey === undefined ? DEFAULT_HOTKEY : opts.defaultHotkey;

  const root = document.createElement("div");
  root.className = "hotkey-recorder";

  const chipText = (hk: string | null) => (hk ? prettyHotkey(hk) : "Not set");

  const chip = document.createElement("kbd");
  chip.textContent = chipText(getCurrent());

  const change = document.createElement("button");
  change.type = "button";
  change.className = "ghost";
  change.textContent = "Change";

  const hint = document.createElement("span");
  hint.className = "meta";

  root.append(chip, change);
  if (defaultHotkey !== null) {
    const reset = document.createElement("button");
    reset.type = "button";
    reset.className = "linklike";
    reset.textContent = "Reset to default";
    reset.addEventListener("click", () => {
      hint.textContent = "";
      save(defaultHotkey);
    });
    root.append(reset);
  }
  if (opts?.allowClear) {
    const clear = document.createElement("button");
    clear.type = "button";
    clear.className = "linklike";
    clear.textContent = "Remove";
    clear.addEventListener("click", () => {
      hint.textContent = "";
      save(null);
    });
    root.append(clear);
  }
  root.append(hint);

  let recording = false;

  const stop = (resume: boolean) => {
    if (!recording) return;
    recording = false;
    root.classList.remove("recording");
    change.textContent = "Change";
    document.removeEventListener("keydown", onKeydown, true);
    window.removeEventListener("blur", onBlur);
    document.removeEventListener(HOTKEY_RECORDER_CANCEL, onCancel);
    if (resume) void invoke("resume_hotkey");
  };

  const save = (accelerator: string | null) => {
    persist(accelerator)
      .then(() => {
        chip.textContent = chipText(accelerator);
        hint.textContent = "Saved ✓";
        // suspend_hotkey parks EVERY app combo (dictation + transforms);
        // the persist call only re-registers what it changed, so resume the
        // rest. resume_hotkey is idempotent — double-registration is safe.
        stop(true);
        if (accelerator !== null && opts?.celebrateOnSave !== false) {
          celebrate(`Hotkey set to ${chipText(accelerator)}.`);
        }
        opts?.onSaved?.(accelerator);
      })
      .catch((err) => {
        hint.textContent = String(err);
        // Rollback re-armed the old combo. Mid-capture, suspend it again so
        // pressing it doesn't fire its action; when idle (reset click), it
        // must stay active.
        if (recording) void invoke("suspend_hotkey");
      });
  };

  const onKeydown = (e: KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "Escape") {
      hint.textContent = "";
      stop(true);
      return;
    }
    const acc = eventToAccelerator(e);
    if (acc === null) return; // bare modifier — keep waiting
    if (acc === "needs-modifier") {
      hint.textContent = "Add a modifier — ⌘, ⌃, ⌥ or ⇧ (with ⇧/⌥ for combos macOS reserves, like ⌘Space).";
      return;
    }
    save(acc);
  };

  const onBlur = () => {
    hint.textContent = "";
    stop(true);
  };

  const onCancel = () => {
    hint.textContent = "";
    stop(true);
  };

  change.addEventListener("click", () => {
    if (recording) {
      hint.textContent = "";
      stop(true);
      return;
    }
    recording = true;
    root.classList.add("recording");
    change.textContent = "Press your shortcut… (Esc to cancel)";
    hint.textContent = "";
    void invoke("suspend_hotkey");
    document.addEventListener("keydown", onKeydown, true);
    window.addEventListener("blur", onBlur);
    document.addEventListener(HOTKEY_RECORDER_CANCEL, onCancel);
  });

  return root;
}
