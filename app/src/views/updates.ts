import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

/** Kept between "check" and "install" so the button installs the same
 *  update it announced instead of re-hitting the network. */
let pending: Update | null = null;
let installing = false;

function setStatus(text: string) {
  $<HTMLParagraphElement>("update-status").textContent = text;
}

function showBadge(show: boolean) {
  $<HTMLSpanElement>("update-badge").classList.toggle("hidden", !show);
}

function setButton(label: string, disabled: boolean) {
  const btn = $<HTMLButtonElement>("update-btn");
  btn.textContent = label;
  btn.disabled = disabled;
}

function showUpdateNotice(version: string) {
  $<HTMLParagraphElement>("un-body").textContent =
    `Update to v${version} now to get the latest features.`;
  $("update-notice").classList.remove("hidden");
}

function hideUpdateNotice() {
  $("update-notice").classList.add("hidden");
}

/** Progress goes to both surfaces while the modal is up, so the user is not
 *  staring at a frozen dialog wondering whether the button worked. */
function setNoticeProgress(text: string) {
  $<HTMLParagraphElement>("un-body").textContent = text;
}

async function doCheck(silent: boolean): Promise<void> {
  if (installing) return;
  if (!silent) setStatus("Checking…");
  try {
    pending = await check();
  } catch (err) {
    // Offline or endpoint unreachable — dictation must not care.
    if (!silent) setStatus(`Couldn't check for updates: ${String(err)}`);
    return;
  }
  if (pending) {
    setButton(`Update to v${pending.version} & restart`, false);
    setStatus(
      pending.body
        ? `v${pending.version} available — ${pending.body}`
        : `v${pending.version} is available.`,
    );
    showBadge(true);
    // The badge alone waited for the user to wander into Settings, so people
    // ran old versions for weeks. Ask them directly instead — but only on the
    // automatic check, so pressing "Check for updates" never throws a modal
    // over the answer the user is already looking at.
    if (silent) showUpdateNotice(pending.version);
  } else {
    setButton("Check for updates", false);
    if (!silent) setStatus("You're on the latest version ✓");
  }
}

async function doInstall(): Promise<void> {
  if (!pending || installing) return;
  installing = true;
  const total = { bytes: 0, len: 0 };
  setButton("Updating…", true);
  $<HTMLButtonElement>("un-update").disabled = true;
  $<HTMLButtonElement>("un-later").disabled = true;
  $("un-error").classList.add("hidden");
  setNoticeProgress("Downloading update…");
  try {
    await pending.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          total.len = event.data.contentLength ?? 0;
          setStatus("Downloading update…");
          break;
        case "Progress": {
          total.bytes += event.data.chunkLength;
          if (total.len > 0) {
            const pct = Math.round((total.bytes / total.len) * 100);
            setStatus(`Downloading update… ${pct}%`);
            setNoticeProgress(`Downloading update… ${pct}%`);
          }
          break;
        }
        case "Finished":
          setStatus("Installing…");
          setNoticeProgress("Installing…");
          break;
      }
    });
    setStatus("Update installed — restarting…");
    setNoticeProgress("Update installed — restarting…");
    await relaunch();
  } catch (err) {
    installing = false;
    setButton(`Update to v${pending.version} & restart`, false);
    setStatus(`Update failed: ${String(err)}. Try again in a minute.`);
    // Leave the modal open on failure — dismissing it would hide the only
    // explanation the user gets for why nothing happened.
    setNoticeProgress(`Update to v${pending.version}.`);
    const error = $("un-error");
    error.textContent = `Update failed: ${String(err)}. Try again in a minute.`;
    error.classList.remove("hidden");
    $<HTMLButtonElement>("un-update").disabled = false;
    $<HTMLButtonElement>("un-later").disabled = false;
  }
}

export function initUpdates(): void {
  void getVersion().then((v) => {
    $<HTMLSpanElement>("app-version").textContent = `v${v}`;
  });

  $<HTMLButtonElement>("update-btn").addEventListener("click", () => {
    if (pending) void doInstall();
    else void doCheck(false);
  });

  $<HTMLButtonElement>("un-update").addEventListener("click", () => void doInstall());
  $<HTMLButtonElement>("un-later").addEventListener("click", () => {
    // Dismissed for this launch only. The badge stays lit, and the next
    // launch asks again — an update nobody installs is the problem this
    // popup exists to fix, so "Later" must not mean "never".
    hideUpdateNotice();
  });

  // Quiet check shortly after launch. It raises the popup as well as the
  // badge; the delay keeps it from landing on top of the first-run wizard.
  window.setTimeout(() => void doCheck(true), 3000);
}
