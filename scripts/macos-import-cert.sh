#!/usr/bin/env bash
# Import the "Purl Self-Signed" code-signing identity into a throwaway
# keychain so `tauri build` can sign the app. Used by the release workflow.
#
# Every release must be signed by the same identity: macOS ties the
# Accessibility grant to it, and an ad-hoc signature changes every build,
# which would silently drop the grant on each update.
#
# Env:
#   MACOS_CERT_P12_BASE64  the exported .p12, base64-encoded (required)
#   MACOS_CERT_PASSWORD    the .p12's export password (required, may be empty)
#   PURL_KEYCHAIN          keychain path (default: $RUNNER_TEMP or $TMPDIR, purl-signing.keychain-db)
#   PURL_SIGNING_IDENTITY  common name to expect (default: "Purl Self-Signed")
#   PURL_KEYCHAIN_SEARCH   "1" (default) adds the keychain to the user search
#                          list so codesign finds it; "0" leaves the list alone
#
# The certificate is self-signed, so codesign only accepts it once it is
# trusted for code signing. On CI (CI=true) that trust is added to the
# System keychain with sudo; elsewhere the script stops and says so.
set -euo pipefail

die() { echo "macos-import-cert: $*" >&2; exit 1; }

[[ -n "${MACOS_CERT_P12_BASE64:-}" ]] || die "MACOS_CERT_P12_BASE64 is not set (see docs/RELEASING.md)"
[[ -n "${MACOS_CERT_PASSWORD+set}" ]] || die "MACOS_CERT_PASSWORD is not set (see docs/RELEASING.md)"

identity="${PURL_SIGNING_IDENTITY:-Purl Self-Signed}"
tmp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
keychain="${PURL_KEYCHAIN:-$tmp/purl-signing.keychain-db}"
keychain_password="$(openssl rand -hex 24)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "$MACOS_CERT_P12_BASE64" | base64 --decode > "$work/cert.p12" \
  || die "MACOS_CERT_P12_BASE64 is not valid base64"

security delete-keychain "$keychain" 2>/dev/null || true
security create-keychain -p "$keychain_password" "$keychain"
# No auto-lock for the length of the build.
security set-keychain-settings "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$work/cert.p12" -k "$keychain" -P "$MACOS_CERT_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security \
  || die "couldn't import the .p12 (wrong MACOS_CERT_PASSWORD?)"
# Let codesign use the key without a GUI prompt.
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null

# The certificate must carry the expected name, or tauri.conf.json's
# signingIdentity won't match it.
security find-certificate -c "$identity" "$keychain" >/dev/null \
  || die "the .p12 has no certificate named \"$identity\""

if [[ "${PURL_KEYCHAIN_SEARCH:-1}" == "1" ]]; then
  # Prepend to the search list, keeping what was there.
  existing=()
  while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%\"}"
    existing+=("${line#\"}")
  done < <(security list-keychains -d user)
  security list-keychains -d user -s "$keychain" "${existing[@]}"
fi

if ! security find-identity -v -p codesigning "$keychain" | grep -qF "\"$identity\""; then
  [[ "${CI:-}" == "true" ]] || die "\"$identity\" is imported but not trusted for code signing.
  In Keychain Access, open the certificate, expand Trust, set Code Signing to Always Trust."
  security find-certificate -c "$identity" -p "$keychain" > "$work/cert.pem"
  sudo security add-trusted-cert -d -r trustRoot -p codeSign \
    -k /Library/Keychains/System.keychain "$work/cert.pem"
  security find-identity -v -p codesigning "$keychain" | grep -qF "\"$identity\"" \
    || die "\"$identity\" is still not a valid code-signing identity after adding trust"
fi

echo "macos-import-cert: \"$identity\" is ready in $keychain"
if [[ -n "${GITHUB_ENV:-}" ]]; then
  echo "PURL_KEYCHAIN=$keychain" >> "$GITHUB_ENV"
fi
