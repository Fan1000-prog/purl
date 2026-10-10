import { invoke } from "@tauri-apps/api/core";
import type { HistoryEntry } from "../types";
import { onViewShow, switchView } from "../main";

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

function relativeTime(unixSecs: number): string {
  const secs = Math.max(0, Math.floor(Date.now() / 1000) - unixSecs);
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

// All content set via textContent — never innerHTML.
function renderItem(e: HistoryEntry): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "history-item";

  const text = document.createElement("p");
  text.className = "history-text";
  text.textContent = e.cleaned || "—";
  li.appendChild(text);

  const meta = document.createElement("div");
  meta.className = "history-meta";
  const when = document.createElement("span");
  when.textContent = relativeTime(e.ts);
  meta.appendChild(when);
  const words = document.createElement("span");
  words.textContent = `${e.words} words`;
  meta.appendChild(words);
  if (e.used_llm) {
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = "AI";
    meta.appendChild(badge);
  }
  const copy = document.createElement("button");
  copy.className = "ghost";
  copy.type = "button";
  copy.textContent = "Copy";
  copy.addEventListener("click", () => {
    void navigator.clipboard.writeText(e.cleaned);
    copy.textContent = "Copied";
    setTimeout(() => (copy.textContent = "Copy"), 1200);
  });
  meta.appendChild(copy);
  li.appendChild(meta);

  const details = document.createElement("details");
  const summary = document.createElement("summary");
  summary.textContent = "Raw Whisper output";
  details.appendChild(summary);
  const raw = document.createElement("p");
  raw.className = "transcript muted";
  raw.textContent = e.raw || "—";
  details.appendChild(raw);
  li.appendChild(details);

  return li;
}

async function refresh() {
  const entries = await invoke<HistoryEntry[]>("get_history");
  const list = $<HTMLUListElement>("history-list");
  list.replaceChildren(...entries.slice().reverse().map(renderItem));
  $<HTMLParagraphElement>("history-empty").classList.toggle(
    "hidden",
    entries.length > 0,
  );
}

export function initHome() {
  document.querySelectorAll<HTMLButtonElement>(".feature-card").forEach((card) => {
    card.addEventListener("click", () => {
      switchView(card.dataset.goto ?? "home");
      const focusId = card.dataset.focus;
      if (focusId) document.getElementById(focusId)?.focus();
    });
  });

  const clearBtn = $<HTMLButtonElement>("clear-history-btn");
  const defaultLabel = clearBtn.textContent ?? "Clear history";
  let armed = false;
  let armTimer: ReturnType<typeof setTimeout> | undefined;

  clearBtn.addEventListener("click", () => {
    if (!armed) {
      // WKWebView can silently no-op window.confirm(), so confirm inline on
      // the button itself instead: first click arms it, second confirms.
      armed = true;
      clearBtn.textContent = "Delete all history? It only lives on this Mac.";
      armTimer = setTimeout(() => {
        armed = false;
        clearBtn.textContent = defaultLabel;
      }, 4000);
      return;
    }
    clearTimeout(armTimer);
    armed = false;
    clearBtn.textContent = defaultLabel;
    invoke("clear_history")
      .then(refresh)
      .catch((err) => {
        $<HTMLParagraphElement>("note").textContent = String(err);
      });
  });
  onViewShow("home", () =>
    void refresh().catch((err) => {
      $<HTMLParagraphElement>("note").textContent = String(err);
    }),
  );
  void refresh().catch((err) => {
    $<HTMLParagraphElement>("note").textContent = String(err);
  });
}
