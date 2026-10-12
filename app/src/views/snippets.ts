import { invoke } from "@tauri-apps/api/core";
import type { Snippet } from "../types";
import { onViewShow } from "../main";
import { celebrate } from "../celebrate";

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

let snippets: Snippet[] = [];
let editingTrigger: string | null = null;

const msg = (text: string) => ($("snippet-msg").textContent = text);

function submitButton(): HTMLButtonElement {
  return $<HTMLFormElement>("snippet-form").querySelector("button[type=submit]") as HTMLButtonElement;
}

function stopEditing() {
  editingTrigger = null;
  $<HTMLFormElement>("snippet-form").reset();
  submitButton().textContent = "Add snippet";
  $("snippet-cancel").classList.add("hidden");
}

async function persist(): Promise<void> {
  await invoke("save_snippets", { list: snippets });
}

function removeSnippet(s: Snippet): void {
  // Locate by reference at click time, never the render-time index: concurrent
  // edits can shift positions before this handler runs.
  const i = snippets.indexOf(s);
  if (i === -1) return;
  snippets.splice(i, 1);
  render(); // optimistic
  persist()
    .then(refresh)
    .catch((err) => {
      msg(String(err));
      void refresh(); // reconverge with disk truth
    });
}

// All content set via textContent, never innerHTML.
function renderItem(s: Snippet): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "list-item snippet-item";

  const trigger = document.createElement("span");
  trigger.className = "item-main";
  trigger.textContent = `"${s.trigger}"`;
  const expansion = document.createElement("span");
  expansion.className = "snippet-expansion muted";
  expansion.textContent = s.expansion;

  const edit = document.createElement("button");
  edit.className = "btn btn-quiet";
  edit.type = "button";
  edit.textContent = "Edit";
  edit.addEventListener("click", () => {
    editingTrigger = s.trigger;
    $<HTMLInputElement>("snippet-trigger").value = s.trigger;
    $<HTMLTextAreaElement>("snippet-expansion").value = s.expansion;
    submitButton().textContent = "Save snippet";
    $("snippet-cancel").classList.remove("hidden");
    $<HTMLInputElement>("snippet-trigger").focus();
  });

  const del = document.createElement("button");
  del.className = "btn btn-quiet";
  del.type = "button";
  del.textContent = "Delete";
  del.setAttribute("aria-label", `Delete snippet ${s.trigger}`);
  del.addEventListener("click", () => removeSnippet(s));

  const actions = document.createElement("div");
  actions.className = "item-actions";
  actions.append(edit, del);
  li.append(trigger, expansion, actions);
  return li;
}

function render() {
  $("snippet-list").replaceChildren(...snippets.map(renderItem));
  $("snippet-empty").classList.toggle("hidden", snippets.length > 0);
}

async function refresh() {
  const error = $("snippet-error");
  try {
    snippets = await invoke<Snippet[]>("get_snippets");
    error.classList.add("hidden");
    $<HTMLFormElement>("snippet-form").hidden = false;
    render();
  } catch (err) {
    snippets = [];
    $("snippet-list").replaceChildren();
    $("snippet-empty").classList.add("hidden");
    // Saving a list we couldn't read would overwrite the file, so no form.
    $<HTMLFormElement>("snippet-form").hidden = true;
    error.textContent = `Couldn't load your snippets (${String(err)}). Try reopening Purl.`;
    error.classList.remove("hidden");
  }
}

export function initSnippets() {
  $("snippet-cancel").addEventListener("click", () => {
    stopEditing();
    msg("");
  });
  $<HTMLFormElement>("snippet-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const trigger = $<HTMLInputElement>("snippet-trigger").value.trim();
    const expansion = $<HTMLTextAreaElement>("snippet-expansion").value.trim();
    if (!trigger || !expansion) {
      msg("Fill in both the trigger and what it expands to.");
      return;
    }
    const editIndex =
      editingTrigger === null
        ? -1
        : snippets.findIndex((x) => x.trigger.toLowerCase() === editingTrigger!.toLowerCase());

    if (snippets.some((x, i) => i !== editIndex && x.trigger.toLowerCase() === trigger.toLowerCase())) {
      msg("A snippet with that trigger already exists.");
      return;
    }

    const previous = editIndex >= 0 ? snippets[editIndex] : null;
    if (editIndex >= 0) snippets[editIndex] = { trigger, expansion };
    else snippets.push({ trigger, expansion });

    persist()
      .then(() => {
        stopEditing();
        msg("Saved.");
        render();
        if (editIndex === -1) celebrate(`"${trigger}" is ready to use.`);
      })
      .catch((err) => {
        if (previous) snippets[editIndex] = previous;
        else snippets.pop();
        msg(String(err));
      });
  });
  onViewShow("snippets", () => void refresh());
  void refresh();
}
