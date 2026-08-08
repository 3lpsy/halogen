#!/usr/bin/env bash
# Upload the signed .ipa to App Store Connect (TestFlight) via Transporter's
# CLI — deliberately separate from signing (uploading publishes). Usage:
# ./upload-apple.sh [-y] [<TART_TAG>]. Needs Transporter.app, ASC_API_KEY_ID +
# ASC_API_ISSUER_ID in .env, and the AuthKey .p8; missing config is an ERROR here.
set -euo pipefail
script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=env.sh
source "${script_dir}/env.sh"

assume_yes="${UPLOAD_YES:-}"
if [ "${1:-}" = "-y" ]; then assume_yes=1; shift; fi

TART_TAG="${TART_TAG:-${1:-$(halogen_default_tag)}}"
out_dir="${script_dir}/artifacts/${TART_TAG}"
ipa="${out_dir}/halogen-${TART_TAG#v}-ios.ipa"
itms="/Applications/Transporter.app/Contents/itms/bin/iTMSTransporter"

[ -f "${ipa}" ] || { echo "error: no signed ipa at ${ipa} — run sign-apple.sh first" >&2; exit 1; }
[ -n "${ASC_API_KEY_ID:-}" ] || { echo "error: ASC_API_KEY_ID not set (.env at repo root)" >&2; exit 1; }
[ -n "${ASC_API_ISSUER_ID:-}" ] || { echo "error: ASC_API_ISSUER_ID not set (.env at repo root)" >&2; exit 1; }
[ -x "${itms}" ] || { echo "error: Transporter.app not installed (Mac App Store) — or drag ${ipa} into its GUI" >&2; exit 1; }
key_file="${HOME}/.appstoreconnect/private_keys/AuthKey_${ASC_API_KEY_ID}.p8"
[ -f "${key_file}" ] || { echo "error: API key not found at ${key_file}" >&2; exit 1; }

# Show exactly what's about to be published, then confirm — uploading is
# the point of no return for a build number.
plist_dir="$(mktemp -d)"
trap 'rm -rf "${plist_dir}"' EXIT
unzip -p "${ipa}" 'Payload/*.app/Info.plist' > "${plist_dir}/Info.plist" 2>/dev/null || true
if [ -s "${plist_dir}/Info.plist" ]; then
  app_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "${plist_dir}/Info.plist" 2>/dev/null || echo '?')"
  app_build="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "${plist_dir}/Info.plist" 2>/dev/null || echo '?')"
  app_bundle="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "${plist_dir}/Info.plist" 2>/dev/null || echo '?')"
else
  app_version='?'; app_build='?'; app_bundle='?'
fi
echo "upload-apple: about to upload to App Store Connect (TestFlight):"
echo "  ipa:     ${ipa}"
echo "  bundle:  ${app_bundle}"
echo "  version: ${app_version} (build ${app_build})"
echo "  sha256:  $(shasum -a 256 "${ipa}" | awk '{print $1}')"
echo "  api key: ${ASC_API_KEY_ID}"
if [ -z "${assume_yes}" ]; then
  printf "Proceed? [y/N] "
  read -r reply
  case "${reply}" in
    y|Y|yes|YES) ;;
    *) echo "upload-apple: aborted — nothing uploaded"; exit 1 ;;
  esac
fi
"${itms}" -m upload -assetFile "${ipa}" \
  -apiKey "${ASC_API_KEY_ID}" -apiIssuer "${ASC_API_ISSUER_ID}"
echo "upload-apple: done — watch TestFlight processing in App Store Connect (~5–15 min)"
