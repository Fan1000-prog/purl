// Which desktop OS the webview runs on. The dev mock can pin it
// (`?mock=1&platform=windows`) so Windows-only UI renders on a Mac.

export type Platform = "macos" | "windows";

export function platform(): Platform {
  return window.__PURL_PLATFORM__ ?? (navigator.userAgent.includes("Windows") ? "windows" : "macos");
}

/** Hotkey as the user sees it: ⌘⇧Space on macOS, Ctrl+Shift+Space on Windows. */
export function displayHotkey(hk: string): string {
  if (platform() === "windows") {
    return hk.replace(/CmdOrCtrl|Command|Cmd|Control/gi, "Ctrl").replace(/Option/gi, "Alt").replace(/Super/gi, "Win");
  }
  return hk
    .replace(/CmdOrCtrl|Command|Cmd|Super/gi, "⌘")
    .replace(/Shift/gi, "⇧")
    .replace(/Option|Alt/gi, "⌥")
    .replace(/Control|Ctrl/gi, "⌃")
    .replace(/\+/g, "");
}

const PRIVACY_URLS: Record<Platform, { mic: string; accessibility: string | null }> = {
  macos: {
    mic: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
    accessibility: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
  },
  windows: { mic: "ms-settings:privacy-microphone", accessibility: null },
};

/** The OS settings page for a permission; null where the OS has no such page. */
export function privacySettingsUrl(kind: "mic" | "accessibility"): string | null {
  return PRIVACY_URLS[platform()][kind];
}
