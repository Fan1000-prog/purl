# Vendored brand

Copied from the private `Fan1000-prog/purl-site` repo, `brand/` folder,
branch `feat/motion-design`, commit `4eafad3`. Do not edit these files here:
change them in purl-site and copy them in again, so the app and the landing
page keep one token set.

Copied: `tokens.css`, `fonts/` (OFL), `components/` (pill, orb and base
styles), and `logo/purl-favicon-*.svg`. Not copied: the style guide page
(`brand.html`, `brand.ts`, `brand.css`), `contrast.ts` (the app has its own check,
`tests/contrast.mjs`, over the pairs it uses), the logo PNGs and the
full-size marks (the app icon sources live in `src-tauri/icons/source/`).

To update: copy the same files from the new commit, update the hash above,
and run `npm test` (contrast + UI).
