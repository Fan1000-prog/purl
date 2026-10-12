import { invoke } from "@tauri-apps/api/core";
import { onViewShow, prettyHotkey } from "../main";
import { createHotkeyRecorder } from "./hotkey-recorder";
import { celebrate } from "../celebrate";
import type { ModelView } from "../types";

export interface Transform {
  id: string;
  name: string;
  description: string;
  prompt: string;
  hotkey: string | null;
  builtin: boolean;
}

let transforms: Transform[] = [];
/** Card currently expanded into its editor; null = all collapsed. */
let editingId: string | null = null;
let loadError: string | null = null;

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

function msg(text: string) {
  $<HTMLParagraphElement>("transforms-msg").textContent = text;
}

async function refresh(): Promise<void> {
  try {
    transforms = await invoke<Transform[]>("get_transforms");
    loadError = null;
  } catch (err) {
    transforms = [];
    loadError = `Couldn't load your transforms (${String(err)}). Try reopening Purl.`;
  }
  render();
  void showSmartNeeded();
}

/** Transforms run on Smart cleanup's model; say so while none is downloaded. */
async function showSmartNeeded() {
  const banner = $("transforms-needs-smart");
  const models = await invoke<ModelView[]>("models_status").catch(() => null);
  const ready = models?.some((m) => m.kind === "llm" && m.state === "ready") ?? true;
  banner.classList.toggle("hidden", ready);
  banner.textContent = "Transforms use Smart cleanup's model. Download it in Settings, under Cleanup, to run them.";
}

/** Save the full list; on failure re-sync from the backend (a hotkey
 *  registration error still persists the list server-side). */
async function persist(next: Transform[]): Promise<void> {
  try {
    await invoke("save_transforms", { list: next });
    transforms = next;
    msg("Saved ✓");
  } catch (err) {
    msg(String(err));
    transforms = await invoke<Transform[]>("get_transforms");
    throw err;
  } finally {
    render();
  }
}

function newId(): string {
  return `custom-${Date.now().toString(36)}`;
}

function field(labelText: string, control: HTMLElement): HTMLLabelElement {
  const label = document.createElement("label");
  label.className = "field";
  const span = document.createElement("span");
  span.textContent = labelText;
  label.append(span, control);
  return label;
}

function buildEditor(t: Transform, isNew: boolean): HTMLElement {
  const editor = document.createElement("div");
  editor.className = "transform-editor";

  const name = document.createElement("input");
  name.value = t.name;
  name.placeholder = "Name";

  const description = document.createElement("input");
  description.value = t.description;
  description.placeholder = "Short description";

  const prompt = document.createElement("textarea");
  prompt.rows = 4;
  prompt.value = t.prompt;
  prompt.placeholder =
    "What should Purl do with the selected text? For example: “Translate to French, keeping the tone.”";

  editor.append(field("Name", name), field("Description", description), field("Instructions", prompt));

  // New transforms don't exist in the backend list yet, so their hotkey
  // can't be registered/persisted until "Add transform" — hold it locally
  // and fold it into the object built on save instead.
  let pendingHotkey: string | null = t.hotkey;

  const recorder = createHotkeyRecorder(
    isNew
      ? {
          get: () => pendingHotkey,
          set: async (acc) => {
            pendingHotkey = acc;
          },
          defaultHotkey: null,
          allowClear: true,
          celebrateOnSave: false,
        }
      : {
          // Hotkey changes persist immediately through the recorder's set().
          get: () => transforms.find((x) => x.id === t.id)?.hotkey ?? null,
          set: async (acc) => {
            await persist(
              transforms.map((x) => (x.id === t.id ? { ...x, hotkey: acc } : x)),
            );
          },
          defaultHotkey: null,
          allowClear: true,
        },
  );
  const hkWrap = document.createElement("div");
  hkWrap.className = "field";
  const hkLabel = document.createElement("span");
  hkLabel.textContent = "Hotkey";
  hkWrap.append(hkLabel, recorder);
  editor.append(hkWrap);

  const actions = document.createElement("div");
  actions.className = "actions";

  const saveBtn = document.createElement("button");
  saveBtn.type = "button";
  saveBtn.className = "btn btn-primary";
  saveBtn.textContent = isNew ? "Add transform" : "Save";
  saveBtn.addEventListener("click", () => {
    const edited: Transform = {
      ...t,
      name: name.value.trim(),
      description: description.value.trim(),
      prompt: prompt.value.trim(),
      hotkey: isNew ? pendingHotkey : t.hotkey,
    };
    const next = isNew
      ? [...transforms, edited]
      : transforms.map((x) => (x.id === t.id ? { ...x, ...edited } : x));
    editingId = null;
    void persist(next)
      .then(() => {
        if (isNew) celebrate(`"${edited.name}" is ready to use.`);
      })
      .catch(() => {
        editingId = t.id; // keep the editor open so nothing typed is lost
        render();
      });
  });
  actions.append(saveBtn);

  if (!t.builtin && !isNew) {
    const del = document.createElement("button");
    del.type = "button";
    del.className = "btn btn-quiet";
    del.textContent = "Delete";
    del.addEventListener("click", () => {
      editingId = null;
      void persist(transforms.filter((x) => x.id !== t.id));
    });
    actions.append(del);
  }

  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "btn btn-quiet";
  cancel.textContent = "Cancel";
  cancel.addEventListener("click", () => {
    editingId = null;
    render();
  });
  actions.append(cancel);

  editor.append(actions);
  // Clicks inside the editor must not toggle the card closed.
  editor.addEventListener("click", (e) => e.stopPropagation());
  return editor;
}

function buildCard(t: Transform): HTMLElement {
  const card = document.createElement("div");
  card.className = "transform-card";
  card.tabIndex = 0;
  card.setAttribute("role", "button");

  const chip = document.createElement("kbd");
  chip.className = "transform-chip";
  chip.textContent = t.hotkey ? prettyHotkey(t.hotkey) : "No hotkey";

  const name = document.createElement("span");
  name.className = "card-title";
  name.textContent = t.name;

  const sub = document.createElement("span");
  sub.className = "card-sub";
  sub.textContent = t.description;

  card.append(chip, name, sub);

  if (editingId === t.id) {
    card.classList.add("editing");
    card.append(buildEditor(t, false));
  } else {
    const open = () => {
      editingId = t.id;
      render();
    };
    card.addEventListener("click", open);
    card.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        open();
      }
    });
  }
  return card;
}

function buildCreateCard(): HTMLElement {
  const card = document.createElement("div");
  card.className = "transform-card transform-create";
  card.tabIndex = 0;
  card.setAttribute("role", "button");

  if (editingId === "new") {
    card.classList.add("editing");
    const title = document.createElement("span");
    title.className = "card-title";
    title.textContent = "New transform";
    card.append(title);
    card.append(
      buildEditor(
        {
          id: newId(),
          name: "",
          description: "",
          prompt: "",
          hotkey: null,
          builtin: false,
        },
        true,
      ),
    );
  } else {
    const plus = document.createElement("span");
    plus.className = "transform-plus";
    plus.textContent = "+";
    const title = document.createElement("span");
    title.className = "card-title";
    title.textContent = "Create your own";
    const sub = document.createElement("span");
    sub.className = "card-sub";
    sub.textContent = "Describe any rewrite and Purl applies it";
    card.append(plus, title, sub);
    const open = () => {
      editingId = "new";
      render();
    };
    card.addEventListener("click", open);
    card.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        open();
      }
    });
  }
  return card;
}

function render() {
  const list = $<HTMLDivElement>("transform-list");
  if (loadError) {
    // Saving over a list we couldn't read would wipe it: no cards, no create.
    list.replaceChildren();
    msg("");
    const error = document.createElement("p");
    error.className = "error-state";
    error.setAttribute("role", "alert");
    error.textContent = loadError;
    list.append(error);
  } else {
    list.replaceChildren(...transforms.map(buildCard), buildCreateCard());
  }
  $("transforms-empty").classList.toggle("hidden", loadError !== null || transforms.length > 0);
  $<HTMLButtonElement>("transforms-reset").hidden = loadError !== null;
}

export function initTransforms() {
  $<HTMLButtonElement>("transforms-reset").addEventListener("click", () => {
    editingId = null;
    invoke<Transform[]>("reset_transforms")
      .then((list) => {
        transforms = list;
        msg("Defaults restored ✓");
        render();
      })
      .catch((err) => msg(String(err)));
  });

  onViewShow("transforms", () => void refresh());
  void refresh();
}
