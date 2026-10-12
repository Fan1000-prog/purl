# Icon sources

All from the locked logo pack, via the brand folder (`purl-site/brand/logo/`).
Dark is Purl's primary surface, so the app icon is the deep variant.

- `purl-icon-deep.svg` / `.png`: the full-square app icon. Every icon in `../`
  except `icon.icns` and the ≤32 px ones comes from its PNG via
  `npx tauri icon src-tauri/icons/source/purl-icon-deep.png -o /tmp/purl-full`.
- `purl-favicon-deep.svg` / `.png`: the bead-only mark (no sound arc), for
  32 px and below: `32x32.png`, `Square30x30Logo.png` and `tray.png` (a copy of
  the 32 px one), from
  `npx tauri icon src-tauri/icons/source/purl-favicon-deep.png -o /tmp/purl-small`.
- `purl-icon-macos.svg` / `.png`: the brand's `purl-app-icon-macos-deep`, the
  deep icon on Apple's icon grid (824 px rounded square on a transparent 1024
  canvas, artwork untouched). Only `icon.icns` is built from it, so the
  Windows `.ico` keeps the full square:

  ```bash
  npx tauri icon src-tauri/icons/source/purl-icon-macos.png -o /tmp/purl-icns
  cp /tmp/purl-icns/icon.icns src-tauri/icons/icon.icns
  ```

- `purl-icon-colour.svg`: the previous fern-field icon, no longer used.
