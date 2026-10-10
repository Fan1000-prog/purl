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
