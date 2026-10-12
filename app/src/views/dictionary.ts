// Dictionary: the user's words (the glossary file) plus, on macOS, the
// corrections Purl learned from their fixes (learning.ts).
//
// The glossary is a text file: one term per line, optional `term = written
// as`, `#` comments (see dictionary.rs). Edits here change single lines and
// keep everything else, comments included, byte for byte.

import { invoke } from "@tauri-apps/api/core";
import { onViewShow } from "../main";
import { initLearning, refreshLearning } from "./learning";

export interface DictionaryEntry {
  term: string;
  writtenAs: string | null;
  /** Index of the line in the file, so removal touches only that line. */
  line: number;
}

export function parseDictionary(text: string): DictionaryEntry[] {
  const entries: DictionaryEntry[] = [];
  text.split("\n").forEach((raw, line) => {
    const l = raw.trim();
    if (!l || l.startsWith("#")) return;
    const eq = l.indexOf("=");
    const term = (eq >= 0 ? l.slice(0, eq) : l).trim();
    if (term) entries.push({ term, writtenAs: eq >= 0 ? l.slice(eq + 1).trim() : null, line });
  });
  return entries;
}

export function addEntry(text: string, term: string, writtenAs: string | null): string {
  const line = writtenAs ? `${term} = ${writtenAs}` : term;
  return text === "" || text.endsWith("\n") ? `${text}${line}\n` : `${text}\n${line}\n`;
}

export function removeLine(text: string, line: number): string {
  const lines = text.split("\n");
  lines.splice(line, 1);
  return lines.join("\n");
}

/** Why a new entry can't be saved, or null if it can. */
export function entryProblem(term: string, writtenAs: string, existing: DictionaryEntry[]): string | null {
  if (!term) return "Type a word or name first.";
  if (term.includes("=") || writtenAs.includes("=")) return "Leave out the = sign.";
  if (term.startsWith("#")) return "A word can't start with #.";
  if (existing.some((e) => e.term.toLowerCase() === term.toLowerCase())) return `"${term}" is already in your dictionary.`;
  return null;
}

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

let text = "";

function msg(t: string) {
  $("dict-msg").textContent = t;
}

function item(e: DictionaryEntry): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "list-item list-row";
  const term = document.createElement("span");
  term.className = "item-main";
  term.textContent = e.term;
  li.append(term);
  if (e.writtenAs) {
    const as = document.createElement("span");
    as.className = "muted";
    as.textContent = `written as ${e.writtenAs}`;
    li.append(as);
  }
  const del = document.createElement("button");
  del.type = "button";
  del.className = "btn btn-quiet";
  del.textContent = "Remove";
  del.setAttribute("aria-label", `Remove ${e.term}`);
  del.addEventListener("click", () => void save(removeLine(text, e.line), `Removed "${e.term}".`));
  li.append(del);
  return li;
}

function render() {
  const entries = parseDictionary(text);
  $("dict-list").replaceChildren(...entries.map(item));
  $("dict-empty").classList.toggle("hidden", entries.length > 0);
}

async function save(next: string, done: string): Promise<boolean> {
  try {
    await invoke("save_glossary", { content: next });
    text = next;
    msg(done);
    render();
    return true;
  } catch (err) {
    msg(`Couldn't save (${String(err)}). Nothing was changed.`);
    return false;
  }
}

async function refresh() {
  const error = $("dict-error");
  try {
    text = await invoke<string>("get_glossary");
    error.classList.add("hidden");
    $<HTMLFormElement>("dict-form").hidden = false;
    render();
  } catch (err) {
    $("dict-list").replaceChildren();
    $("dict-empty").classList.add("hidden");
    // Saving over a file we couldn't read would wipe it, so the form goes too.
    $<HTMLFormElement>("dict-form").hidden = true;
    error.textContent = `Couldn't load your dictionary (${String(err)}). Try reopening Purl.`;
    error.classList.remove("hidden");
  }
}

export function initDictionary() {
  $<HTMLFormElement>("dict-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const termInput = $<HTMLInputElement>("dict-term");
    const asInput = $<HTMLInputElement>("dict-expansion");
    const term = termInput.value.trim();
    const writtenAs = asInput.value.trim();
    const problem = entryProblem(term, writtenAs, parseDictionary(text));
    if (problem) {
      msg(problem);
      termInput.focus();
      return;
    }
    void save(addEntry(text, term, writtenAs || null), `Added "${term}".`).then((ok) => {
      if (!ok) return;
      termInput.value = "";
      asInput.value = "";
      termInput.focus();
    });
  });
  initLearning();
  onViewShow("dictionary", () => {
    void refresh();
    void refreshLearning();
  });
  void refresh();
}
