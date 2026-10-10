# AGENTS.md

Purl: a free, MIT, local-first dictation app for macOS and Windows (Tauri v2,
Rust core in `app/src-tauri`, TypeScript + Vite UI in `app/src`).

- Spec: `docs/specs/2026-10-09-purl-design.md`
- Plan: `docs/plans/2026-10-09-purl-v1.md`

## Commands

| What | Command |
|---|---|
| Build the app | `cd app && npm ci && npm run tauri build` |
| Rust tests | `cd app/src-tauri && cargo test` |
| Lint | `cd app/src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings` |
| UI build | `cd app && npm run build` |

## Conventions

- Bundle id `app.purl.desktop`; crate `purl`, lib `purl_lib`.
- No network at dictation time. Network is only for model downloads and the
  update check.
- The updater private key never goes in the repo.
- Example text says "Google Chat", never "Slack".
