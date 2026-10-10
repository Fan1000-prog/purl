// After setup, Home offers Smart cleanup once: an optional local language
// model that fixes grammar and phrasing. Dismissing it is a per-device
// preference, so it lives in localStorage rather than settings.json.

import { invoke } from "@tauri-apps/api/core";
import { mountModelDownload } from "../components/model-download";
import type { ModelView } from "../types";

const SMART_MODEL = "qwen3.5-2b";
const DISMISSED_KEY = "purl.smartOfferDismissed";

function dismissed(): boolean {
  try {
    return localStorage.getItem(DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

function remember() {
  try {
    localStorage.setItem(DISMISSED_KEY, "1");
  } catch {
    // Storage blocked: the card simply shows again next launch.
  }
}

export async function showSmartOffer() {
  const slot = document.getElementById("home-offer");
  if (!slot || dismissed() || document.getElementById("smart-offer")) return;
  const models = await invoke<ModelView[]>("models_status").catch(() => []);
  if (models.find((m) => m.id === SMART_MODEL)?.state === "ready") return;

  const card = document.createElement("section");
  card.id = "smart-offer";
  card.className = "card smart-offer";
  card.setAttribute("aria-labelledby", "smart-offer-title");

  const close = document.createElement("button");
  close.type = "button";
  close.className = "smart-offer-close";
  close.setAttribute("aria-label", "Dismiss");
  close.textContent = "×";

  const title = document.createElement("h2");
  title.id = "smart-offer-title";
  title.textContent = "Want smarter cleanup?";

  const text = document.createElement("p");
  text.className = "muted";
  text.textContent =
    "Smart cleanup adds a small language model that fixes grammar and phrasing, on your computer. It's optional, and Fast cleanup works without it.";

  const downloadSlot = document.createElement("div");
  card.append(close, title, text, downloadSlot);
  slot.append(card);

  const download = mountModelDownload(downloadSlot, {
    id: SMART_MODEL,
    title: "Smart cleanup",
    detail: "optional",
    onReady: () => {
      text.textContent = "Smart cleanup is downloaded. Turn it on in Settings, under Cleanup.";
    },
  });

  close.addEventListener("click", () => {
    remember();
    download.destroy();
    card.remove();
  });
}
