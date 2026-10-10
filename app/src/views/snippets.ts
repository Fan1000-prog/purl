import { invoke } from "@tauri-apps/api/core";
import type { Snippet } from "../types";
import { onViewShow } from "../main";
import { celebrate } from "../celebrate";

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

let snippets: Snippet[] = [];
let editingTrigger: string | null = null;

function submitButton(): HTMLButtonElement {
  return $<HTMLFormElement>("snippet-form").querySelector(
    "button[type=submit]",
  ) as HTMLButtonElement;
}

async function persist(): Promise<void> {
  await invoke("save_snippets", { list: snippets });
}

function removeSnippet(s: Snippet): void {
  // Locate by reference at click time — never trust the render-time index,
  // since concurrent edits can shift array positions before this handler runs.
  const i = snippets.indexOf(s);
  if (i === -1) return;
  snippets.splice(i, 1);
  render(); // optimistic update
  persist()
    .then(refresh)
    .catch((err) => {
      $<HTMLSpanElement>("snippet-msg").textContent = String(err);
      void refresh(); // reconverge memory/UI with disk truth
    });
}

// All content set via textContent — never innerHTML.
function renderItem(s: Snippet): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "snippet-item";

  const trigger = document.createElement("span");
  trigger.className = "snippet-trigger";
  trigger.textContent = `"${s.trigger}"`;
  li.appendChild(trigger);

  const arrow = document.createElement("span");
  arrow.className = "muted";
  arrow.textContent = "→";
  li.appendChild(arrow);

  const expansion = document.createElement("span");
  expansion.className = "snippet-expansion";
  expansion.textContent = s.expansion;
  li.appendChild(expansion);

  const edit = document.createElement("button");
  edit.className = "ghost";
  edit.type = "button";
  edit.textContent = "Edit";
  edit.addEventListener("click", () => {
    editingTrigger = s.trigger;
    $<HTMLInputElement>("snippet-trigger").value = s.trigger;
    $<HTMLTextAreaElement>("snippet-expansion").value = s.expansion;
    submitButton().textContent = "Save snippet";
    $<HTMLInputElement>("snippet-trigger").focus();
  });
  li.appendChild(edit);

  const del = document.createElement("button");
  del.className = "ghost";
  del.type = "button";
  del.textContent = "Delete";
  del.addEventListener("click", () => {
    removeSnippet(s);
  });
  li.appendChild(del);

  return li;
}

function render() {
  $<HTMLUListElement>("snippet-list").replaceChildren(
    ...snippets.map(renderItem),
  );
}

async function refresh() {
  snippets = await invoke<Snippet[]>("get_snippets");
  render();
}

export function initSnippets() {
  $<HTMLFormElement>("snippet-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const msg = $<HTMLSpanElement>("snippet-msg");
    const trigger = $<HTMLInputElement>("snippet-trigger").value.trim();
    const expansion = $<HTMLTextAreaElement>("snippet-expansion").value.trim();
    if (!trigger || !expansion) {
      msg.textContent = "Both fields are required.";
      return;
    }
    const editIndex =
      editingTrigger === null
        ? -1
        : snippets.findIndex(
            (x) => x.trigger.toLowerCase() === editingTrigger!.toLowerCase(),
          );

    if (
      snippets.some(
        (x, i) => i !== editIndex && x.trigger.toLowerCase() === trigger.toLowerCase(),
      )
    ) {
      msg.textContent = "A snippet with that trigger already exists.";
      return;
    }

    const previous = editIndex >= 0 ? snippets[editIndex] : null;
    if (editIndex >= 0) {
      snippets[editIndex] = { trigger, expansion };
    } else {
      snippets.push({ trigger, expansion });
    }

    const isNewSnippet = editIndex === -1;
    persist()
      .then(() => {
        editingTrigger = null;
        msg.textContent = "Saved.";
        submitButton().textContent = "Add snippet";
        $<HTMLFormElement>("snippet-form").reset();
        render();
        if (isNewSnippet) celebrate(`"${trigger}" is ready to use.`);
      })
      .catch((err) => {
        if (editIndex >= 0 && previous) {
          snippets[editIndex] = previous;
        } else {
          snippets.pop();
        }
        msg.textContent = String(err);
      });
  });
  onViewShow("snippets", () => void refresh());
  void refresh();
}
