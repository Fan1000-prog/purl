// The stats row on Home: today's words next to the all-time picture.

import { invoke } from "@tauri-apps/api/core";
import type { Stats } from "../types";

function isoDay(d: Date): string {
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${m}-${day}`;
}

function stat(value: string, label: string): HTMLElement {
  const box = document.createElement("div");
  box.className = "stat";
  const num = document.createElement("span");
  num.className = "stat-num";
  num.textContent = value;
  const name = document.createElement("span");
  name.className = "stat-label";
  name.textContent = label;
  box.append(num, name);
  return box;
}

export async function renderStats(el: HTMLElement): Promise<void> {
  try {
    const s = await invoke<Stats>("get_stats");
    const minutes = s.total_record_ms / 60000;
    el.replaceChildren(
      stat((s.per_day[isoDay(new Date())] ?? 0).toLocaleString(), "words today"),
      stat(s.total_words.toLocaleString(), "words all time"),
      stat(minutes > 0 ? String(Math.round(s.total_words / minutes)) : "—", "words per minute"),
      stat(s.total_sessions.toLocaleString(), "dictations"),
    );
  } catch (err) {
    const msg = document.createElement("p");
    msg.className = "error-state";
    msg.setAttribute("role", "alert");
    msg.textContent = `Couldn't load your stats (${String(err)}).`;
    el.replaceChildren(msg);
  }
}
