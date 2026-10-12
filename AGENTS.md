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
| Release script tests | `node --test scripts/*.test.mjs` |
| Release smoke check | `cd app/src-tauri && cargo run -- --smoke` |
| Fetch llama-server | `cd app && npm run fetch-llama` (runs before `tauri dev`/`build`; pinned tag + sha256 in `scripts/fetch-llama.mjs`) |
| Real-model tests | `cargo run --example fetch_models [model-id]`, then `cargo test -- --ignored` |

## Conventions

- Bundle id `app.purl.desktop`; crate `purl`, lib `purl_lib`.
- No network at dictation time. Network is only for model downloads and the
  update check. Speech runs in-process; the Smart cleanup `llama-server`
  sidecar is spoken to over `127.0.0.1` only, with a per-run API key.
- Text from other apps (transform selections, fields read back for
  correction learning) is untrusted. It goes into an LLM prompt only inside
  delimited tags it cannot close, and the prompt says it is data.
- The updater private key never goes in the repo.
- Releases: see `docs/RELEASING.md`. Asset names are version-free and
  defined once in `scripts/release-assets.mjs`.
- Example text says "Google Chat", never "Slack".
