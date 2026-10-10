// The "what's new" card, shown once after an update.
//
// It carries the field-reading disclosure as well as the feature list, and
// that is deliberate. The first-run wizard is the strongest thing this app
// says about reading other applications' text fields, and it only runs for
// new users — everyone already on a previous version updates straight into
// the behaviour. Putting the disclosure in the card they meet immediately
// after updating reaches exactly the people the wizard misses, at the moment
// it starts being true for them.
//
// Shown when the running version differs from the last one acknowledged, or
// when the disclosure was never acknowledged at all (an install that updated
// into field reading before this card existed). Dismissing records both.

import { getVersion } from "@tauri-apps/api/app";
import { getSettings, saveVersionSeen } from "../main";
import { platform } from "../platform";

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

function close(version: string) {
  saveVersionSeen(version)
    .then(() => $("field-notice").classList.add("hidden"))
    .catch((err) => {
      // Keep the card up rather than pretending it was acknowledged: nothing
      // persisted, so it would reappear next launch anyway.
      const error = $("fn-error");
      error.textContent = `Couldn't save that (${String(err)}). Close and reopen the app, or turn the feature off in Settings → Learn corrections.`;
      error.classList.remove("hidden");
    });
}

function show(version: string) {
  // Correction learning (and so field reading) exists only on macOS.
  $("fn-learning").hidden = platform() !== "macos";
  // Nothing left to tell on this platform: skip the card rather than show
  // an empty one.
  if (![...document.querySelectorAll<HTMLElement>("#field-notice .fn-news")].some((p) => !p.hidden)) return;
  $<HTMLButtonElement>("fn-dismiss").addEventListener("click", () =>
    close(version),
  );
  $("field-notice").classList.remove("hidden");
}

export function initFieldNotice() {
  const s = getSettings();
  // New users are told all of this in the wizard, which stamps the version
  // and the disclosure flag when it finishes — they must not get the card on
  // top of it.
  if (s?.onboarding_complete !== true) return;

  // Field reading (correction learning) only exists on macOS.
  const owesDisclosure = platform() === "macos" && s.field_reading_disclosed === false;

  void getVersion()
    .then((version) => {
      if (s.last_seen_version !== version || owesDisclosure) {
        show(version);
      }
    })
    .catch(() => {
      // Version unreadable. Fall back to the disclosure alone — that is the
      // part that must not be skipped. Recording an empty version leaves the
      // card due again next launch, which is the safe direction to fail.
      if (owesDisclosure) show("");
    });
}
