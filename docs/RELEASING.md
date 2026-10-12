# Releasing Purl

A release is one version tag. Pushing `vX.Y.Z` (or `vX.Y.Z-rc.N`) runs
[`.github/workflows/release.yml`](../.github/workflows/release.yml), which:

1. checks the tag matches `version` in `app/src-tauri/tauri.conf.json`,
2. builds, signs and smoke-tests on `macos-14` (Apple Silicon) and
   `windows-latest` (x64), with read-only repo access,
3. in one separate `publish` job (the only one that can write), creates a
   **draft** GitHub Release (a pre-release when the tag has a `-`) and
   uploads the installers, updater files and `latest.json`.

Nothing is public until a person publishes the draft.

| Asset | What it is |
|---|---|
| `Purl_aarch64.dmg` | macOS installer, Apple Silicon |
| `Purl_x64-setup.exe` | Windows installer, per-user, unsigned in v1 |
| `Purl_aarch64.app.tar.gz` (+ `.sig`) | what the macOS updater downloads |
| `Purl_x64-setup.exe.sig` | updater signature for the Windows installer |
| `latest.json` | updater manifest: `darwin-aarch64` and `windows-x86_64` |

Asset names carry no version, so the landing page can link to
`https://github.com/Fan1000-prog/purl/releases/latest/download/Purl_aarch64.dmg`
and `.../Purl_x64-setup.exe`. GitHub's `latest` skips drafts and
pre-releases: those links (and the in-app updater) only see a published,
non-pre-release version. To try a release candidate, use
`.../releases/download/v0.9.0-rc.1/<asset>` instead.

An `-rc` build reports the plain version from `tauri.conf.json` (a
`v0.9.0-rc.1` build says `0.9.0`). The updater only offers a *newer*
version, so people on an rc are **not** offered the final `0.9.0` release.
They have to install it by hand, or wait for `0.9.1`.

The workflow can also be started by hand (**Actions → release → Run
workflow**, once it is on `main`). A manual run builds and smoke-tests with a
throwaway macOS certificate and no updater key, keeps the installers as
workflow artifacts, and never publishes, even when started on a tag: only a
pushed tag gets the secrets. Pull requests never trigger it, so the signing
secrets never reach PR code.

## One-time setup: signing secrets

The workflow needs three repository secrets. Only the repo owner can set
them, and only once.

| Secret | What |
|---|---|
| `MACOS_CERT_P12_BASE64` | the "Purl Self-Signed" certificate and key, as base64 |
| `MACOS_CERT_PASSWORD` | the password you give the `.p12` when exporting it |
| `TAURI_SIGNING_PRIVATE_KEY` | the updater key, `~/.tauri/purl.key` |

`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is not needed: `purl.key` has no
password, and the workflow passes an empty one when the secret is unset.
(GitHub won't store an empty secret anyway.)

### Why a self-signed certificate

macOS remembers the Accessibility permission (which paste needs) per signing
identity. An ad-hoc signature is different on every build, so each update
would silently drop the permission and paste would stop working. Signing
every release with the same certificate keeps the permission across updates.
It is not Apple notarization: macOS still blocks the first open (see
[First open](#first-open-macos-and-windows)).

**Keep the certificate.** If it is lost and replaced, every user has to grant
Accessibility again after the next update. Save the `.p12` and its password
in your password manager.

**Keep the updater key too.** Back up `~/.tauri/purl.key` in your password
manager as well. Installed copies trust only the public key built into them,
so if the private key is lost, they can never update themselves again. Never
replace the key without first shipping a release that people install by hand,
and that release must carry the new public key.

### 1. Create the certificate (Mac, Keychain Access)

1. Open **Keychain Access** (Spotlight: "Keychain Access").
2. Menu bar: **Keychain Access → Certificate Assistant → Create a Certificate…**
3. Fill in:
   - **Name:** `Purl Self-Signed` (exactly; the build looks for this name)
   - **Identity Type:** Self-Signed Root
   - **Certificate Type:** Code Signing
   - Tick **Let me override defaults**.
4. Click **Continue**. On the warning about a self-signed certificate, click
   **Continue** again.
5. **Validity Period (days):** `3650`. Click **Continue**.
6. Personal information: put anything in **Email Address** (it can stay
   blank) and leave **Common Name** as `Purl Self-Signed`. Click **Continue**.
7. **Key Pair Information:** leave RSA, 2048 bits. Click **Continue**.
8. **Key Usage Extension:** leave the defaults (Signature ticked). Click
   **Continue**.
9. **Extended Key Usage Extension:** make sure **Code Signing** is ticked.
   Click **Continue**.
10. Click **Continue** through the remaining screens without changes until
    **Specify a Location For The Certificate**: Keychain **login**. Click
    **Create**, then **Done**.

Optional, only if you want to build signed releases on this Mac: double-click
the new certificate, expand **Trust**, set **Code Signing** to **Always
Trust**, close the window and enter your Mac password. CI adds this trust on
its own.

### 2. Export it as a .p12

1. In Keychain Access, choose the **login** keychain and the **My
   Certificates** tab.
2. Right-click **Purl Self-Signed** → **Export "Purl Self-Signed"…**
3. **File Format:** Personal Information Exchange (.p12). Save as
   `purl-signing.p12` on the Desktop.
4. Set a strong password and type it twice. This is `MACOS_CERT_PASSWORD`.
5. Enter your Mac login password when asked, and click **Always Allow** or
   **Allow**.

### 3. Set the GitHub secrets

In Terminal, from any folder (`gh` must be logged in as the repo owner):

```bash
base64 -i ~/Desktop/purl-signing.p12 | gh secret set MACOS_CERT_P12_BASE64 --repo Fan1000-prog/purl
gh secret set MACOS_CERT_PASSWORD --repo Fan1000-prog/purl
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo Fan1000-prog/purl < ~/.tauri/purl.key
```

The second command asks for the value: paste the `.p12` password and press
Enter. Nothing is echoed to the screen or written to your shell history.

Or in the browser: **github.com/Fan1000-prog/purl → Settings → Secrets and
variables → Actions → New repository secret**, once per secret. For
`MACOS_CERT_P12_BASE64`, paste the output of
`base64 -i ~/Desktop/purl-signing.p12 | pbcopy` (it's on the clipboard).
For `TAURI_SIGNING_PRIVATE_KEY`, paste the output of `pbcopy < ~/.tauri/purl.key`.

Check they're there (values are never shown):

```bash
gh secret list --repo Fan1000-prog/purl
```

Then move `purl-signing.p12` off the Desktop and into your password manager.

## Cutting a release

0. **Before the first release candidate**, start a manual run from `main`
   (**Actions → release → Run workflow**) and wait for both jobs to pass.
   It builds and installs the real Windows installer with
   `onnxruntime.dll` and `llama-server`, and signs the macOS app the way a
   release does, without publishing anything.
1. Set the new version in all three places, on a branch, and merge it:
   `app/src-tauri/tauri.conf.json`, `app/src-tauri/Cargo.toml`,
   `app/package.json`. The tag must match `tauri.conf.json` (a `-rc.N`
   suffix on the tag is allowed).
2. Tag `main` and push the tag:

   ```bash
   git checkout main && git pull
   git tag v0.9.0-rc.1
   git push origin v0.9.0-rc.1
   ```

3. Watch the run: `gh run watch --repo Fan1000-prog/purl`. Both build jobs
   must pass their smoke test. Then `publish` creates the draft release and
   uploads the installers, updater files and `latest.json`.
4. **Install the rc on real hardware before publishing.** Download from the
   draft (**Releases → the draft → Assets**), not from `latest`.
   - Mac: open `Purl_aarch64.dmg`, drag Purl to Applications, and open it
     (see [First open](#first-open-macos-and-windows)). Grant the
     microphone and Accessibility, then dictate into Gmail in Chrome and
     check the text pastes. Turn on Smart cleanup once and check it works.
   - Windows, if you have a PC: run `Purl_x64-setup.exe`, get past
     SmartScreen, and dictate the same way.
5. Open the draft under **Releases**, check the assets listed above are all
   there, and edit the notes. Say plainly that Windows has only been
   smoke-tested on CI until someone tries it on real hardware, that there is
   no Intel Mac build, and paste the first-open steps below. Then publish.

If a run fails while the release is still a draft, fix the cause and re-run
the failed jobs; uploads overwrite (`--clobber`). Once a release is
published, `publish` refuses to touch it: cut a new tag instead. To redo a
tag from scratch, delete the draft release and the tag first.

## First open (macOS and Windows)

Purl isn't notarized by Apple or signed for Windows, so both systems block
the first open. Put these steps in every release's notes.

- **macOS 14:** in Applications, right-click Purl → **Open**, then **Open**
  again.
- **macOS 15 and later:** right-click → Open no longer works. Open Purl once
  and close the warning. Then go to **System Settings → Privacy & Security**,
  scroll down to the message about Purl, click **Open Anyway** and confirm
  with your password.
- **Windows:** on the blue "Windows protected your PC" screen, click **More
  info → Run anyway**.

After the first open, both systems remember the choice.

## What the smoke test proves

`purl --smoke` (see `app/src-tauri/src/smoke.rs`) starts the installed app
headless, loads settings, the dictionary and the platform layer without
models, writes `SMOKE OK <version> ...` to `<temp>/purl-smoke.log` and exits.
On Windows the job installs silently (`/S`), runs it, checks
`onnxruntime.dll` sits beside `purl.exe`, and uninstalls. On macOS it runs from the mounted DMG and checks the signature
is "Purl Self-Signed".

It does not prove dictation works: no microphone, no model, no paste into a
real app. That needs a person on real hardware.
