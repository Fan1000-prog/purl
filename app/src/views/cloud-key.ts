import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { splitToken, providerLabel, type Engine } from "../engine-token";

/** Where a user gets a key, per provider. Gemini first: its free tier is
 *  the reason this feature is reachable without a purchase order. */
const SIGNUP_URL: Record<string, string> = {
  gemini: "https://aistudio.google.com/apikey",
  claude: "https://console.anthropic.com/settings/keys",
};

const SIGNUP_LABEL: Record<string, string> = {
  gemini: "Get a free API key from Google AI Studio →",
  claude: "Get an API key from the Anthropic Console →",
};

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

export async function hasKey(engine: Engine): Promise<boolean> {
  if (engine === "local") return true;
  return invoke<boolean>("cloud_key_status", { engine });
}

/** The durable disclosure. The modal is seen once; this setting persists
 *  indefinitely, so the row states the privacy tradeoff for as long as a
 *  cloud engine is selected. */
export function renderCloudRow(statusEl: HTMLElement, token: string): void {
  const mine = ++renderEpoch;
  const { engine, model } = splitToken(token);
  if (engine === "local") return;
  const label = providerLabel(engine);

  statusEl.replaceChildren();
  const line = document.createElement("span");
  line.className = "meta";
  statusEl.appendChild(line);

  const addButton = (text: string, onClick: () => void) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "ghost";
    b.textContent = text;
    b.addEventListener("click", onClick);
    statusEl.appendChild(b);
    return b;
  };

  void hasKey(engine).then((saved) => {
    if (mine !== renderEpoch) return; // a newer render superseded this one
    line.textContent = saved
      ? `${label} · key saved ✓ · transcript text leaves your Mac`
      : `${label} · no key saved · cleanup will fall back to your local model`;

    addButton("Test", () => {
      line.textContent = `Testing ${label}…`;
      invoke("test_cloud_key", { engine, model })
        .then(() => {
          line.textContent = `${label} · key works ✓ · transcript text leaves your Mac`;
        })
        .catch((err) => {
          line.textContent = String(err);
        });
    });
    addButton(saved ? "Change" : "Add key", () => {
      void openModal(engine, model).then(() => renderCloudRow(statusEl, token));
    });
    if (saved) {
      addButton("Remove", () => {
        invoke("clear_cloud_api_key", { engine })
          .then(() => renderCloudRow(statusEl, token))
          .catch((err) => {
            line.textContent = String(err);
          });
      });
    }
  });
}

/** Opens the modal when the selected cloud engine has no key yet.
 *  Resolves false when the user cancels, so the caller can revert the
 *  dropdown rather than leave a keyless cloud engine selected. */
export async function ensureKey(token: string): Promise<boolean> {
  const { engine, model } = splitToken(token);
  if (engine === "local") return true;
  if (await hasKey(engine)) return true;
  return openModal(engine, model);
}

/** One modal at a time. The backdrop stops mouse clicks but does not make the
 *  rest of the page inert, so Tab can still reach the <select> behind it and a
 *  second change event would re-enter `openModal`. Both promises would then
 *  share one input and one set of buttons: a single Save click would fire both
 *  handlers, and whichever closed first would detach only its own listeners. */
let modalOpen = false;

/** Guards against overlapping renders. `renderCloudRow` clears the row
 *  synchronously but fills it from an async `hasKey` round trip, so arrowing
 *  through the dropdown could leave one provider's line above another's
 *  buttons — and "Remove" would then clear the wrong provider's key. */
let renderEpoch = 0;

/** Resolves true when a key was saved and verified, false on cancel.
 *  `model` is the id the user actually selected, so the verification covers
 *  the real selection rather than a stand-in. */
function openModal(engine: Engine, model: string): Promise<boolean> {
  if (modalOpen) return Promise.resolve(false);
  modalOpen = true;
  const label = providerLabel(engine);
  const backdrop = $<HTMLDivElement>("cloud-key-modal");
  const input = $<HTMLInputElement>("ck-input");
  const error = $<HTMLParagraphElement>("ck-error");
  const progress = $<HTMLSpanElement>("ck-progress");
  const save = $<HTMLButtonElement>("ck-save");
  const cancel = $<HTMLButtonElement>("ck-cancel");
  const getKey = $<HTMLButtonElement>("ck-get-key");

  $<HTMLHeadingElement>("ck-title").textContent = `Connect ${label}`;
  $<HTMLParagraphElement>("ck-intro").textContent =
    `${label} runs cleanup in the cloud using your own API key.`;
  $<HTMLParagraphElement>("ck-privacy").textContent =
    `Your transcript text is sent to ${label} using this key. Audio never leaves your Mac. The key is stored in your macOS Keychain.`;
  getKey.textContent = SIGNUP_LABEL[engine] ?? "Get an API key →";

  input.value = "";
  error.textContent = "";
  error.classList.add("hidden");
  progress.textContent = "";
  backdrop.classList.remove("hidden");
  input.focus();

  return new Promise<boolean>((resolve) => {
    const close = (result: boolean) => {
      modalOpen = false;
      backdrop.classList.add("hidden");
      // The key must not linger in the DOM after the modal closes.
      input.value = "";
      save.removeEventListener("click", onSave);
      cancel.removeEventListener("click", onCancel);
      getKey.removeEventListener("click", onGetKey);
      document.removeEventListener("keydown", onKeydown);
      resolve(result);
    };

    const fail = (msg: string) => {
      error.textContent = msg;
      error.classList.remove("hidden");
      progress.textContent = "";
      save.disabled = false;
    };

    async function onSave() {
      // Enter calls this programmatically, and a disabled button does not
      // block a direct call — without this, holding Enter during a test fires
      // repeated save/test round trips against the provider.
      if (save.disabled) return;
      const key = input.value.trim();
      if (!key) {
        fail("Paste your API key first.");
        return;
      }
      save.disabled = true;
      error.classList.add("hidden");
      progress.textContent = "Testing…";
      try {
        await invoke("set_cloud_api_key", { engine, key });
      } catch (err) {
        fail(String(err));
        return;
      }
      try {
        // Verify against the model the user actually selected, not a stand-in:
        // a valid key paired with a bad model id should fail here rather than
        // at the user's next dictation.
        await invoke("test_cloud_key", { engine, model });
      } catch (err) {
        // Never keep a key that just failed its own test — and if the removal
        // itself fails, say so. Swallowing it would tell the user their key was
        // rejected while a rejected key stayed in their Keychain.
        try {
          await invoke("clear_cloud_api_key", { engine });
          fail(String(err));
        } catch (clearErr) {
          fail(
            `${err}. The key could not be removed either (${clearErr}) — open Keychain Access and delete it manually.`,
          );
        }
        return;
      }
      save.disabled = false;
      close(true);
    }

    function onCancel() {
      close(false);
    }

    function onGetKey() {
      void openUrl(SIGNUP_URL[engine] ?? "https://aistudio.google.com/apikey");
    }

    function onKeydown(e: KeyboardEvent) {
      if (e.key === "Escape") close(false);
      if (e.key === "Enter" && document.activeElement === input) void onSave();
    }

    save.addEventListener("click", onSave);
    cancel.addEventListener("click", onCancel);
    getKey.addEventListener("click", onGetKey);
    document.addEventListener("keydown", onKeydown);
  });
}
