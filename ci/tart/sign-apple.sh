#!/usr/bin/env bash
# Sign the collected iOS .app (→ .ipa) + mac .app ON THE HOST (no Xcode
# needed); config from .env, unconfigured → notice + exit 0. Usage:
# ./sign-apple.sh [<release-dir>]. macOS notarization is still TODO
# (docs/internal/TART.md).
set -euo pipefail
script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=env.sh
source "${script_dir}/env.sh"

TART_TAG="${TART_TAG:-$(halogen_default_tag)}"
release_dir="${1:-${script_dir}/artifacts/${TART_TAG}/release}"
[ -d "${release_dir}" ] || { echo "error: no artifacts at ${release_dir} — run build-releases.sh first" >&2; exit 1; }
out_dir="$(cd "${release_dir}/.." && pwd)"

if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  echo "sign-apple: APPLE_SIGNING_IDENTITY not set (.env at repo root) — skipping; artifacts stay unsigned."
  exit 0
fi

# ── Optional: import the .p12 into a throwaway keychain ──────────────────────
# Omit APPLE_P12_PATH if the identity already lives in your login keychain.
keychain=""
cleanup() {
  if [ -n "${keychain}" ]; then
    security list-keychains -d user -s login.keychain-db >/dev/null 2>&1 || true
    security delete-keychain "${keychain}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
if [ -n "${APPLE_P12_PATH:-}" ]; then
  keychain="$(mktemp -d)/halogen-ci.keychain-db"
  security create-keychain -p ci "${keychain}"
  security set-keychain-settings "${keychain}"     # no auto-lock timeout
  security unlock-keychain -p ci "${keychain}"
  security import "${APPLE_P12_PATH}" -P "${APPLE_P12_PASSWORD:-}" -A -t cert -f pkcs12 -k "${keychain}"
  # The WWDR intermediates normally arrive with Xcode, which this host lacks —
  # fetch + import them (best-effort so an offline re-sign still works).
  wwdr_dir="$(dirname "${keychain}")"
  for ca in AppleWWDRCAG2 AppleWWDRCAG3 AppleWWDRCAG4 AppleWWDRCAG5 AppleWWDRCAG6 AppleWWDRCAG7 AppleWWDRCAG8; do
    if curl -fsSL --max-time 15 "https://www.apple.com/certificateauthority/${ca}.cer" -o "${wwdr_dir}/${ca}.cer" 2>/dev/null; then
      security import "${wwdr_dir}/${ca}.cer" -k "${keychain}" -t cert >/dev/null 2>&1 || true
    fi
  done
  # Let Apple's signing tools use the key without a UI prompt.
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k ci "${keychain}" >/dev/null
  security list-keychains -d user -s "${keychain}" login.keychain-db
fi
sign_args=()
[ -n "${keychain}" ] && sign_args+=(--keychain "${keychain}")

# ── iOS: embed profile, sign, package .ipa ───────────────────────────────────
ios_app="$(find "${release_dir}/ios" -maxdepth 4 -name '*.app' -type d 2>/dev/null | head -n1 || true)"
if [ -n "${ios_app}" ]; then
  echo "sign-apple: iOS app: ${ios_app}"

  # ── Supply-chain gate ──────────────────────────────────────────────────
  # The build VM is untrusted (that's why signing is host-side) — refuse
  # anything not shaped exactly like Halogen before the key touches it.
  bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "${ios_app}/Info.plist")"
  if [ "${bundle_id}" != "org.fgsec.halogen" ]; then
    echo "error: gate: bundle id '${bundle_id}' != org.fgsec.halogen" >&2; exit 1
  fi
  # Halogen links its Rust core statically — ANY embedded binary is hostile.
  for bad in Frameworks PlugIns Watch; do
    if [ -e "${ios_app}/${bad}" ]; then
      echo "error: gate: unexpected ${bad}/ in app (static-only build)" >&2; exit 1
    fi
  done
  if find "${ios_app}" \( -name '*.dylib' -o -name '*.framework' -o -name '*.appex' \) 2>/dev/null | grep -q .; then
    echo "error: gate: embedded binaries found — Halogen ships none" >&2; exit 1
  fi
  machos="$(find "${ios_app}" -type f -exec file -b {} + | grep -c 'Mach-O' || true)"
  if [ "${machos}" != "1" ]; then
    echo "error: gate: ${machos} Mach-O files in app, expected exactly 1 (the executable)" >&2; exit 1
  fi
  # The VM must not supply its own profile (ours is embedded below). A
  # profile byte-identical to OURS is fine — that's a previous sign attempt
  # of this same artifact (signing must be re-runnable after a failure).
  if [ -e "${ios_app}/embedded.mobileprovision" ]; then
    if [ -n "${IOS_PROVISIONING_PROFILE:-}" ] \
      && cmp -s "${ios_app}/embedded.mobileprovision" "${IOS_PROVISIONING_PROFILE}"; then
      echo "sign-apple: note: embedded.mobileprovision from a previous sign attempt (matches ours)"
    else
      echo "error: gate: unexpected embedded.mobileprovision (not ours) — refusing" >&2; exit 1
    fi
  fi
  # Privacy surface must not grow silently: a trojan wanting camera/mic/
  # location needs a usage string; the only one Halogen declares is
  # local-network. Background modes stay exactly [audio].
  info_json="$(plutil -convert json -o - "${ios_app}/Info.plist")"
  rogue="$(printf '%s' "${info_json}" | grep -o '"NS[A-Za-z]*UsageDescription"' | grep -v '"NSLocalNetworkUsageDescription"' || true)"
  if [ -n "${rogue}" ]; then
    echo "error: gate: unexpected usage-description keys: ${rogue}" >&2; exit 1
  fi
  if ! printf '%s' "${info_json}" | grep -q '"UIBackgroundModes":\["audio"\]'; then
    echo "error: gate: UIBackgroundModes is not exactly [audio]" >&2; exit 1
  fi
  echo "sign-apple: gate passed (bundle id, static-only, 1 Mach-O, no VM profile, privacy surface)"

  if [ -n "${IOS_PROVISIONING_PROFILE:-}" ]; then
    cp "${IOS_PROVISIONING_PROFILE}" "${ios_app}/embedded.mobileprovision"
  else
    echo "sign-apple: WARNING — IOS_PROVISIONING_PROFILE not set; device installs need an embedded profile"
  fi
  # Device installs need the app signed WITH the entitlements from the
  # provisioning profile (application-identifier, team, get-task-allow) —
  # extract them from the profile's signed plist.
  ent_args=()
  if [ -n "${IOS_PROVISIONING_PROFILE:-}" ]; then
    ent_dir="$(mktemp -d)"
    security cms -D -i "${IOS_PROVISIONING_PROFILE}" > "${ent_dir}/profile.plist"
    /usr/libexec/PlistBuddy -x -c 'Print :Entitlements' "${ent_dir}/profile.plist" \
      > "${ent_dir}/entitlements.plist"
    ent_args=(--entitlements "${ent_dir}/entitlements.plist")
  else
    echo "sign-apple: WARNING — no profile, signing without entitlements; device installs will fail validation"
  fi
  # ${arr[@]+...}: bash-3.2-safe expansion of a possibly-empty array under set -u.
  codesign --force --sign "${APPLE_SIGNING_IDENTITY}" \
    ${sign_args[@]+"${sign_args[@]}"} ${ent_args[@]+"${ent_args[@]}"} "${ios_app}"
  codesign --verify --strict "${ios_app}"
  ipa="${out_dir}/halogen-${TART_TAG#v}-ios.ipa"
  staging="$(mktemp -d)"
  mkdir "${staging}/Payload"
  cp -R "${ios_app}" "${staging}/Payload/"
  rm -f "${ipa}"
  (cd "${staging}" && zip -qry "${ipa}" Payload)
  rm -rf "${staging}"
  # Audit trail: what exactly got signed, reconstructable later.
  shasum -a 256 "${ipa}" | tee -a "${out_dir}/SHA256SUMS"
  echo "sign-apple: wrote ${ipa}"
  echo "sign-apple: upload is a separate, explicit step: ./upload-apple.sh [${TART_TAG}]"
else
  echo "sign-apple: no iOS .app under ${release_dir}/ios — skipping iOS"
fi

# ── macOS: sign the desktop app ──────────────────────────────────────────────
mac_app="$(find "${release_dir}" -name '*.app' -type d ! -path '*/ios/*' 2>/dev/null | head -n1 || true)"
if [ -n "${mac_app}" ]; then
  echo "sign-apple: macOS app: ${mac_app}"
  # TODO(decide): personal-Mac use needs only an Apple Development (or ad-hoc)
  # signature; public distribution needs Developer ID + hardened runtime +
  # notarization (notarytool with an ASC API key).
  codesign --force --deep --sign "${APPLE_SIGNING_IDENTITY}" ${sign_args[@]+"${sign_args[@]}"} "${mac_app}"
  echo "sign-apple: signed ${mac_app} (left in place)"
else
  echo "sign-apple: no macOS .app under ${release_dir} — skipping macOS"
fi

echo "sign-apple: done"
