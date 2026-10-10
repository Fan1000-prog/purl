# Icon sources

- `purl-icon-colour.svg`: the full-square app icon from the logo pack. Every
  icon in `../` except `icon.icns` comes from its PNG via
  `npx tauri icon <purl-icon-colour.png>`; at 32 px and below the bead-only
  favicon is used instead (`32x32.png`, `Square30x30Logo.png`, `tray.png`).
- `purl-icon-macos.svg` / `.png`: the same artwork on Apple's icon grid
  (824 px rounded square, 185 px radius, transparent 1024 canvas). Only
  `icon.icns` is built from it, so the Windows `.ico` keeps the full square:

  ```bash
  npx tauri icon src-tauri/icons/source/purl-icon-macos.png -o /tmp/purl-icns
  cp /tmp/purl-icns/icon.icns src-tauri/icons/icon.icns
  ```
