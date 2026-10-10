// "Learned from your fixes" on the Dictionary screen: corrections Purl picked
// up by reading the field it pasted into. macOS only in v1, so the section is
// hidden where learning isn't supported.

import { invoke } from "@tauri-apps/api/core";
import type { LearnedRule } from "../types";
import { confirmButton } from "../components/confirm-button";
import { getStatus } from "../status";

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

let rules: LearnedRule[] = [];

// Show the rule the way the user made it: the phrase, not the field names.
function phrase(r: LearnedRule): string {
  const before = r.before === "^" ? "" : r.before;
  return [before, r.to, r.after].filter(Boolean).join(" ");
}

function showError(text: string) {
  const el = $("learned-msg");
  el.textContent = text;
  el.classList.toggle("hidden", text === "");
}

// All content set via textContent, never innerHTML.
function renderItem(r: LearnedRule): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "list-item list-row";

  const heard = document.createElement("span");
  heard.className = "muted";
  heard.textContent = `"${r.from}" →`;
  const fixed = document.createElement("span");
  fixed.className = "item-main";
  fixed.textContent = phrase(r);
  li.append(heard, fixed);

  if (!r.active) {
    const pending = document.createElement("span");
    pending.className = "muted";
    pending.textContent = "applies after one more fix";
    li.append(pending);
  }

  const del = document.createElement("button");
  del.className = "btn btn-quiet";
  del.type = "button";
  del.textContent = "Forget";
  del.addEventListener("click", () => {
    // Locate by reference at click time, never the render-time index.
    const i = rules.indexOf(r);
    if (i === -1) return;
    rules.splice(i, 1);
    render(); // optimistic
    invoke("delete_learned_rule", { from: r.from, to: r.to, before: r.before })
      .then(refreshLearning)
      .catch((err) => {
        showError(String(err));
        void refreshLearning(); // reconverge with disk truth
      });
  });
  li.append(del);
  return li;
}

function render(): void {
  $("learned-list").replaceChildren(...rules.map(renderItem));
  $("learned-empty").classList.toggle("hidden", rules.length > 0);
  $("learned-clear").hidden = rules.length === 0;
}

export async function refreshLearning(): Promise<void> {
  try {
    rules = await invoke<LearnedRule[]>("get_learned_rules");
    showError("");
    render();
  } catch (err) {
    rules = [];
    $("learned-list").replaceChildren();
    $("learned-empty").classList.add("hidden");
    showError(`Couldn't load learned corrections (${String(err)}).`);
  }
}

export function initLearning(): void {
  confirmButton($("learned-clear"), "Forget every correction?", () => {
    invoke("clear_learned_rules")
      .then(refreshLearning)
      .catch((err) => showError(String(err)));
  });
  void getStatus().then(
    (s) => ($("learned-section").hidden = !s.learningSupported),
    () => {},
  );
  void refreshLearning();
}
