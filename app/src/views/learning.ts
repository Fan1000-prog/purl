import { invoke } from "@tauri-apps/api/core";
import type { LearnedRule } from "../types";
import { onViewShow } from "../main";

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

let rules: LearnedRule[] = [];

// Show the rule the way the user made it: the phrase, not the field names.
function phrase(r: LearnedRule): string {
  const before = r.before === "^" ? "" : r.before;
  return [before, r.to, r.after].filter(Boolean).join(" ");
}

// All content set via textContent — never innerHTML.
function renderItem(r: LearnedRule): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "snippet-item";

  const heard = document.createElement("span");
  heard.className = "snippet-trigger";
  heard.textContent = `"${r.from}"`;
  li.appendChild(heard);

  const arrow = document.createElement("span");
  arrow.className = "muted";
  arrow.textContent = "→";
  li.appendChild(arrow);

  const fixed = document.createElement("span");
  fixed.className = "snippet-expansion";
  fixed.textContent = phrase(r);
  li.appendChild(fixed);

  if (!r.active) {
    const pending = document.createElement("span");
    pending.className = "muted";
    pending.textContent = "· learns after one more";
    pending.title =
      "Corrections that could be a one-off edit start applying once they've been made twice.";
    li.appendChild(pending);
  }

  const del = document.createElement("button");
  del.className = "ghost";
  del.type = "button";
  del.textContent = "Forget";
  del.addEventListener("click", () => {
    // Locate by reference at click time — never trust the render-time index.
    const i = rules.indexOf(r);
    if (i === -1) return;
    rules.splice(i, 1);
    render(); // optimistic
    invoke("delete_learned_rule", { from: r.from, to: r.to, before: r.before })
      .then(refresh)
      .catch((err) => {
        $<HTMLSpanElement>("learned-msg").textContent = String(err);
        void refresh(); // reconverge with disk truth
      });
  });
  li.appendChild(del);

  return li;
}

function render(): void {
  $<HTMLUListElement>("learned-list").replaceChildren(...rules.map(renderItem));
  $<HTMLParagraphElement>("learned-empty").classList.toggle(
    "hidden",
    rules.length > 0,
  );
}

async function refresh(): Promise<void> {
  rules = await invoke<LearnedRule[]>("get_learned_rules");
  render();
}

export function initLearning(): void {
  const clearBtn = $<HTMLButtonElement>("learned-clear");
  const defaultLabel = clearBtn.textContent ?? "Clear all";
  let armed = false;
  let armTimer: ReturnType<typeof setTimeout> | undefined;

  // WKWebView can silently no-op window.confirm(), so confirm on the button
  // itself — the same two-step the Clear history button uses.
  clearBtn.addEventListener("click", () => {
    if (!armed) {
      armed = true;
      clearBtn.textContent = "Forget every correction?";
      armTimer = setTimeout(() => {
        armed = false;
        clearBtn.textContent = defaultLabel;
      }, 4000);
      return;
    }
    clearTimeout(armTimer);
    armed = false;
    clearBtn.textContent = defaultLabel;
    invoke("clear_learned_rules")
      .then(refresh)
      .catch((err) => {
        $<HTMLSpanElement>("learned-msg").textContent = String(err);
      });
  });

  onViewShow("settings", () => void refresh().catch(() => {}));
  void refresh().catch(() => {});
}
