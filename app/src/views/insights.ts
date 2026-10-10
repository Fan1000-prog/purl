import { invoke } from "@tauri-apps/api/core";
import type { Stats } from "../types";
import { onViewShow } from "../main";

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

/** Range dropdown value → number of trailing days to render. */
const RANGE_DAYS: Record<string, number> = {
  day: 1,
  week: 7,
  month: 30,
  year: 365,
};

let lastPerDay: Record<string, number> = {};

function isoDay(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** 0–3 intensity bucket relative to the max day in range. */
function bucket(words: number, max: number): number {
  if (words === 0 || max === 0) return 0;
  if (words >= max * 0.66) return 3;
  if (words >= max * 0.33) return 2;
  return 1;
}

function renderHeatmap(perDay: Record<string, number>, rangeDays: number) {
  const container = $<HTMLDivElement>("heatmap");
  const cells: HTMLSpanElement[] = [];

  // Trailing `rangeDays` days, column-major (grid-auto-flow: column), ending
  // today. Pad the first column so today lands on the correct weekday row.
  const today = new Date();
  const days = rangeDays + (rangeDays > 1 ? 6 - today.getDay() : 0);
  const dates: Date[] = [];
  for (let i = days - 1; i >= 0; i--) {
    const d = new Date(today);
    d.setDate(today.getDate() - i);
    dates.push(d);
  }
  const max = Math.max(0, ...dates.map((d) => perDay[isoDay(d)] ?? 0));
  for (const d of dates) {
    const words = perDay[isoDay(d)] ?? 0;
    const cell = document.createElement("span");
    cell.className = `heat h${bucket(words, max)}`;
    cell.title = `${isoDay(d)}: ${words} words`;
    cells.push(cell);
  }
  container.replaceChildren(...cells);
}

function currentRangeDays(): number {
  const range = $<HTMLSelectElement>("heatmap-range").value;
  return RANGE_DAYS[range] ?? RANGE_DAYS.month;
}

async function refresh() {
  const s = await invoke<Stats>("get_stats");
  $<HTMLSpanElement>("stat-words").textContent = s.total_words.toLocaleString();
  const minutes = s.total_record_ms / 60000;
  $<HTMLSpanElement>("stat-wpm").textContent =
    minutes > 0 ? String(Math.round(s.total_words / minutes)) : "—";
  $<HTMLSpanElement>("stat-sessions").textContent =
    s.total_sessions.toLocaleString();
  $<HTMLSpanElement>("stat-cleanups").textContent =
    s.llm_cleanups.toLocaleString();
  lastPerDay = s.per_day;
  renderHeatmap(lastPerDay, currentRangeDays());
}

export function initInsights() {
  $<HTMLSelectElement>("heatmap-range").addEventListener("change", () => {
    renderHeatmap(lastPerDay, currentRangeDays());
  });
  onViewShow("insights", () => void refresh().catch(() => {}));
  void refresh().catch(() => {});
}
