#!/usr/bin/env bash
# Flatpak from PREBUILT pieces via plain file/ostree plumbing — no
# flatpak-builder (its bwrap can't nest in this runner) and no sandbox needed.
# Inputs: a stage dir (app, libs, .desktop, metainfo, icon) + the GNOME
# runtimes installed. FLATPAK_GNOME_BRANCH must match setup-flatpak.sh.

set -euo pipefail

stage="${1:?usage: build-flatpak.sh <stage-dir> <out-bundle>}"
out="${2:?usage: build-flatpak.sh <stage-dir> <out-bundle>}"

APP_ID=org.fgsec.halogen
RUNTIME_BRANCH="${FLATPAK_GNOME_BRANCH:-49}"
FFMPEG_BRANCH="${FLATPAK_FFMPEG_BRANCH:-25.08}"

# ── Preflight: every NEEDED library must resolve inside the runtime ─────────
# Static stand-in for `flatpak run … ldd` (bwrap is blocked here) — catches a
# runtime that dropped webkit2gtk at build time, not on users' machines.
runtime_files="${FLATPAK_USER_DIR:-$HOME/.local/share/flatpak}/runtime/org.gnome.Platform/x86_64/${RUNTIME_BRANCH}/active/files"
if [ ! -d "${runtime_files}" ]; then
  echo "::error::org.gnome.Platform//${RUNTIME_BRANCH} not installed (expected ${runtime_files}) — flatpak install flathub org.gnome.Platform//${RUNTIME_BRANCH} (CI: ci/internal/setup-flatpak.sh)" >&2
  exit 1
fi
missing=0
for elf in "${stage}/halogen-ui" "${stage}"/*.so.*; do
  for lib in $(objdump -p "${elf}" | awk '/NEEDED/{print $2}'); do
    # Bundled next to the app (e.g. libxdo.so.3) — resolves via /app/lib.
    [ -e "${stage}/${lib}" ] && continue
    if ! find "${runtime_files}/lib" -maxdepth 3 -name "${lib}" -print -quit 2>/dev/null | grep -q .; then
      echo "::error::${lib} (needed by $(basename "${elf}")) not found in org.gnome.Platform//${RUNTIME_BRANCH} — bundle it or bump the runtime" >&2
      missing=1
    fi
  done
done
[ "${missing}" = 0 ] || exit 1

# ── Assemble ─────────────────────────────────────────────────────────────────
build=flatpak-build
repo=flatpak-repo
rm -rf "${build}" "${repo}"

flatpak build-init "${build}" "${APP_ID}" org.gnome.Sdk org.gnome.Platform "${RUNTIME_BRANCH}"

install -Dm755 "${stage}/halogen-ui" "${build}/files/bin/halogen-ui"
# Styles are embedded in the binary (no assets dir; restore the dx assets copy
# if a runtime asset!() ever returns). libxdo isn't in the GNOME runtime —
# bundle it; /app/lib is on the library path.
install -Dm644 "${stage}/libxdo.so.3" "${build}/files/lib/libxdo.so.3"
install -Dm644 "${stage}/${APP_ID}.desktop" "${build}/files/share/applications/${APP_ID}.desktop"
install -Dm644 "${stage}/${APP_ID}.metainfo.xml" "${build}/files/share/metainfo/${APP_ID}.metainfo.xml"
install -Dm644 "${stage}/icon-512.png" "${build}/files/share/icons/hicolor/512x512/apps/${APP_ID}.png"
# Mount point for the ffmpeg extension (AAC/m4a decode via the runtime's
# gst-libav; mp3/ogg/opus/flac are covered by the runtime itself).
mkdir -p "${build}/files/lib/ffmpeg"

flatpak build-finish "${build}" \
  --command=halogen-ui \
  --share=network \
  --share=ipc \
  --socket=fallback-x11 \
  --socket=wayland \
  --socket=pulseaudio \
  --device=dri \
  --extension="org.freedesktop.Platform.ffmpeg-full=directory=lib/ffmpeg" \
  --extension="org.freedesktop.Platform.ffmpeg-full=version=${FFMPEG_BRANCH}" \
  --extension="org.freedesktop.Platform.ffmpeg-full=add-ld-path=."

# build-export validates the exported icon by re-exec'ing its validator under
# $FLATPAK_BWRAP — which can't work here (masked /proc, see header). Point it
# at the noop shim: validation still runs, just unsandboxed (trusted input —
# our own icon).
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export FLATPAK_BWRAP="${script_dir}/bwrap-noop.sh"

flatpak build-export "${repo}" "${build}"
# --runtime-repo lets `flatpak install ./bundle` auto-fetch the GNOME runtime
# + the ffmpeg extension from Flathub on user machines.
flatpak build-bundle \
  --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "${repo}" "${out}" "${APP_ID}"

echo "built ${out}"
