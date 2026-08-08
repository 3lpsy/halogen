#!/usr/bin/env bash
# AppImage from the same prebuilt stage the flatpak uses — lays out an AppDir
# and squashes it (no dependency discovery). APPIMAGE_EXTRACT_AND_RUN=1 avoids
# FUSE; --runtime-file uses the baked type2 runtime so nothing downloads.
# webkit2gtk/gtk3 come from the host distro — the flatpak is the bundled answer.

set -euo pipefail

stage="${1:?usage: build-appimage.sh <stage-dir> <out-appimage>}"
out="${2:?usage: build-appimage.sh <stage-dir> <out-appimage>}"

APP_ID=org.fgsec.halogen
RUNTIME_FILE="${APPIMAGE_RUNTIME_FILE:-/usr/local/lib/appimage/runtime-x86_64}"

if [ ! -f "${RUNTIME_FILE}" ]; then
  echo "::error::AppImage type2 runtime not found at ${RUNTIME_FILE} — bake it into the toolchain image (ci/docker/toolchain/Dockerfile) or point APPIMAGE_RUNTIME_FILE at one" >&2
  exit 1
fi

appdir=appimage-build/AppDir
rm -rf appimage-build
mkdir -p "${appdir}"

install -Dm755 "${stage}/halogen-ui" "${appdir}/usr/bin/halogen-ui"
# Styles are embedded in the binary (no assets dir; restore the dx assets copy
# if a runtime asset!() ever returns). libxdo is NEEDED — bundle it in usr/lib.
install -Dm644 "${stage}/libxdo.so.3" "${appdir}/usr/lib/libxdo.so.3"
install -Dm644 "${stage}/${APP_ID}.desktop" "${appdir}/usr/share/applications/${APP_ID}.desktop"
install -Dm644 "${stage}/icon-512.png" "${appdir}/usr/share/icons/hicolor/512x512/apps/${APP_ID}.png"
# appimagetool contract: desktop file + icon (matching its Icon= key) at the
# AppDir root, .DirIcon for file managers.
cp "${stage}/${APP_ID}.desktop" "${appdir}/${APP_ID}.desktop"
cp "${stage}/icon-512.png" "${appdir}/${APP_ID}.png"
ln -sf "${APP_ID}.png" "${appdir}/.DirIcon"

cat > "${appdir}/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="${HERE}/usr/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
exec "${HERE}/usr/bin/halogen-ui" "$@"
EOF
chmod 0755 "${appdir}/AppRun"

# ARCH: the payload entrypoint is a shell script, so appimagetool can't infer
# the architecture from it — set it explicitly.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 appimagetool \
  --runtime-file "${RUNTIME_FILE}" \
  "${appdir}" "${out}"

echo "built ${out}"
