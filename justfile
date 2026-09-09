# Halogen task runner. Recipes grouped by prefix; `just` (no args) lists them.

# Load the repo-root .env (if present) into every recipe; shell env wins over it.
set dotenv-load := true

# ── Variables ─────────────────────────────────────────────────────────────────

# Dev data: db + logs (git-ignored).
devdata := justfile_directory() / "data" / "run" / "dev"
# Built web frontend (git-ignored contents; embedded into the server binary).
dist := justfile_directory() / "dist"
uidir := justfile_directory() / "webui" / "app"
# Cross-platform app icon set (see data/assets/generate-icons.py).
assets := justfile_directory() / "data" / "assets"
# All webui packages, for the wasm-target lint pass.
webui_packages := "-p halogen-apiclient -p halogen-webui -p halogen-webui-accounts -p halogen-webui-app-state -p halogen-webui-cache-purge -p halogen-webui-commands -p halogen-webui-component-forms -p halogen-webui-component-icons -p halogen-webui-component-loading -p halogen-webui-component-navigation -p halogen-webui-component-toast -p halogen-webui-component-widgets -p halogen-webui-config -p halogen-webui-episode-actions -p halogen-webui-episode-list -p halogen-webui-episode-list-page -p halogen-webui-hook-auth-redirect -p halogen-webui-hook-confirm-action -p halogen-webui-hook-context -p halogen-webui-hook-deep-link-fetch -p halogen-webui-hook-dom-stream -p halogen-webui-hook-episodes -p halogen-webui-hook-form -p halogen-webui-hook-is-admin -p halogen-webui-hook-latest-wins -p halogen-webui-hook-list-view-state -p halogen-webui-hook-paged-pool -p halogen-webui-hook-pull-to-refresh -p halogen-webui-hook-row-memory -p halogen-webui-hook-scroll-memory -p halogen-webui-hook-window-event -p halogen-webui-hooks -p halogen-webui-library-widgets -p halogen-webui-listview -p halogen-webui-logging -p halogen-webui-media -p halogen-webui-page-app -p halogen-webui-page-login -p halogen-webui-platform -p halogen-webui-player -p halogen-webui-player-backend -p halogen-webui-player-controls -p halogen-webui-player-media-session -p halogen-webui-player-sleep -p halogen-webui-player-types -p halogen-webui-provider-accounts -p halogen-webui-provider-app -p halogen-webui-provider-config -p halogen-webui-provider-connection -p halogen-webui-provider-discover -p halogen-webui-provider-downloads -p halogen-webui-provider-episode -p halogen-webui-provider-history -p halogen-webui-provider-local -p halogen-webui-provider-playback -p halogen-webui-provider-player -p halogen-webui-provider-playlist -p halogen-webui-provider-podcast -p halogen-webui-provider-session -p halogen-webui-provider-sync -p halogen-webui-provider-toast -p halogen-webui-provider-webview-media -p halogen-webui-routes -p halogen-webui-store -p halogen-webui-store-idb -p halogen-webui-sync-engine -p halogen-webui-transport-ws -p halogen-webui-view-admin-users -p halogen-webui-view-cache-control -p halogen-webui-view-config -p halogen-webui-view-config-overrides -p halogen-webui-view-configure-swipes -p halogen-webui-view-discover -p halogen-webui-view-dock-config -p halogen-webui-view-downloads -p halogen-webui-view-episode -p halogen-webui-view-history -p halogen-webui-view-home -p halogen-webui-view-latest -p halogen-webui-view-logs -p halogen-webui-view-menu -p halogen-webui-view-not-found -p halogen-webui-view-playlists -p halogen-webui-view-podcasts -p halogen-webui-view-polling -p halogen-webui-view-queue -p halogen-webui-view-server-errors -p halogen-webui-view-settings -p halogen-webui-view-user-edit -p halogen-webui-worker"
# Cargo target dir (honors CARGO_TARGET_DIR).
cargo_target := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() / "target")
# podman if present, else docker; override with CONTAINER_ENGINE=docker.
engine := env("CONTAINER_ENGINE", `command -v podman >/dev/null 2>&1 && echo podman || echo docker`)
# Git-ignored scratch for `just ios-sign-local`: downloads, the signed IPA, nothing
# else. Apple credentials stay at their configured paths and are never copied here.
signing_dir := env("HALOGEN_SIGNING_DIR", justfile_directory() / "data" / "signing")
# Apple's Linux Transporter, unpacked once by `just ios-transporter-setup`. It is
# not baked into the toolchain image the way rcodesign is: its licence binds only
# once a person accepts it, and forbids copying it where others can use it.
transporter_dir := env("HALOGEN_TRANSPORTER_DIR", signing_dir / "transporter")
# CI only: an absolute directory outside the runner's per-run checkout to build
# from, so compiler paths repeat across runs. Empty = build in the checkout.
work_dir := env("HALOGEN_WORK_DIR", "")

# Service endpoints: explicit knob (.env.example) wins, else derived from
# SERVICES_ROOT_DOMAIN — no hostname lives in the repo.
services_root := env("SERVICES_ROOT_DOMAIN", "")
# deps-proxy crates index. The registry NAME must match the `replace-with` the
# build image bakes into $CARGO_HOME/config.toml (chilled-proxy) or cargo errors.
export CARGO_REGISTRIES_CHILLED_PROXY_INDEX := env("CARGO_REGISTRIES_CHILLED_PROXY_INDEX", if services_root == "" { "" } else { "sparse+https://deps." + services_root + "/crates/index/" })
# Toolchain image ref for docker-* runs (empty → those recipes fail with a clear
# message). `export TOOLCHAIN_IMAGE=halogen-toolchain:local` after docker-build-toolchain.
toolchain_image := env("TOOLCHAIN_IMAGE", if services_root == "" { "" } else { "registry." + services_root + "/halogen-toolchain:latest" })
# Base image for docker-build-toolchain: explicit CI_BASE_IMAGE wins, else
# derived from the root domain like the other service endpoints.
ci_base_image := env("CI_BASE_IMAGE", if services_root == "" { "" } else { "registry." + services_root + "/ci-podman-dx-win:latest" })

export npm_config_registry := env("npm_config_registry", env("NPM_REGISTRY", if services_root == "" { "https://npm-registry-not-configured.invalid/" } else { "https://deps." + services_root + "/npm/" }))

# ── Default ───────────────────────────────────────────────────────────────────

# Default: `just` with no args lists recipes.
default:
	@just --list

# Shared implementation behind version-bump-*. part ∈ {major,minor,patch}.
_version-bump part:
	#!/usr/bin/env bash
	set -euo pipefail
	part='{{part}}'
	cur="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/')"
	if [[ ! "$cur" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
	  echo "error: [workspace.package] version '$cur' is not a plain X.Y.Z" >&2
	  exit 1
	fi
	major="${BASH_REMATCH[1]}"; minor="${BASH_REMATCH[2]}"; patch="${BASH_REMATCH[3]}"
	case "$part" in
	  major) major=$((major + 1)); minor=0; patch=0 ;;
	  minor) minor=$((minor + 1)); patch=0 ;;
	  patch) patch=$((patch + 1)) ;;
	  *) echo "error: unknown bump part '$part'" >&2; exit 1 ;;
	esac
	new="${major}.${minor}.${patch}"
	awk -v new="$new" '
	  /^\[/ { inpkg = ($0 == "[workspace.package]") }
	  inpkg && /^version[[:space:]]*=/ && !done { sub(/"[^"]*"/, "\"" new "\""); done = 1 }
	  { print }
	' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
	echo "[workspace.package] version: ${cur} -> ${new}"
	echo "Next: cargo build (refresh Cargo.lock), commit, then 'just ci-tagged-release'."

# ── Compile checks (check-*) ──────────────────────────────────────────────────

# Native workspace compile check.
check:
	cargo check

# The wire graph must stay wasm-safe (the web/PWA client depends on it).
check-wasm:
	cargo check -p halogen-wire --target wasm32-unknown-unknown

# A `mod tests` in a crate outside default-members is compiled by the wasm
# clippy pass but never *run* by any tier. Fail rather than let it rot.
# webui_packages is a hand-kept -p list; a crate missing from it skips its wasm
# clippy silently. Fail loudly instead (CI-011).
check-webui-packages:
	#!/usr/bin/env bash
	set -euo pipefail
	# Not under webui/, but deliberately in the list: it compiles for wasm too.
	allowed_extra="halogen-apiclient"
	listed="$(printf '%s\n' {{webui_packages}} | grep -v '^-p$' | sort -u)"
	actual="$(for c in webui/*/Cargo.toml; do
	  sed -n 's/^name = "\([^"]*\)"/\1/p' "$c" | head -n1
	done | sort -u)"
	fail=0
	while read -r crate; do
	  [ -n "$crate" ] || continue
	  printf '%s\n' "${listed}" | grep -qxF "$crate" && continue
	  echo "error: ${crate} is missing from webui_packages — its wasm clippy never runs" >&2
	  fail=1
	done <<< "${actual}"
	while read -r crate; do
	  [ -n "$crate" ] || continue
	  printf '%s\n' "${actual}" | grep -qxF "$crate" && continue
	  printf '%s\n' "${allowed_extra}" | grep -qxF "$crate" && continue
	  echo "error: webui_packages names ${crate}, which is not a crate under webui/" >&2
	  fail=1
	done <<< "${listed}"
	[ "$fail" = 0 ] || { echo "fix: update webui_packages in the justfile" >&2; exit 1; }
	echo "webui_packages names every crate under webui/ ($(printf '%s\n' "${actual}" | wc -l))"

check-tests-run:
	python3 ci/internal/runner/check-test-members.py

# Everything CI gates on, in CI's order. This is the single definition:
# ci/docker/testing/run-tests.sh calls it rather than restating the tiers.
check-all: fmt-check clippy check check-wasm check-tests-run check-webui-packages test-apple-release ios-parse ios-lint test-crates test-webui test-doc test-integ test-e2e

# ── Lint / format ─────────────────────────────────────────────────────────────

clippy:
	cargo clippy --all-targets -- -D warnings
	# --all-targets or a `mod tests` that no longer compiles passes silently:
	# webui crates are wasm-only, so this is the only pass that sees them.
	cargo clippy {{webui_packages}} --target wasm32-unknown-unknown --all-targets -- -D warnings

fmt:
	cargo fmt --all

# ── Swift (iOS) ───────────────────────────────────────────────────────────────
# The xtool darwin SDK gives Linux the full dev loop: `ios-check` typechecks,
# `ios-device-test` builds the dev app + UI tests and runs them on the dev
# phone. CI (ios-task) is the fallback and owns sim/unit/release lanes.

# The darwin SDK must carry the HOST clang builtin headers, not Apple's —
# with Apple's, every SwiftUI/UIKit interface build fails and parallel builds
# look deadlocked (FEAT-048). Idempotent; keeps the originals as a .bak.
ios-sdk-fix:
	#!/usr/bin/env bash
	set -euo pipefail
	tc="$(readlink -f "$HOME/.swiftpm/swift-sdks/darwin.artifactbundle")/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/clang"
	ver="$(find "$tc" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | head -1)"
	host="$(swift -print-target-info | python3 -c 'import sys,json;print(json.load(sys.stdin)["paths"]["runtimeResourcePath"])')/clang/include"
	if diff -q "$tc/$ver/include/arm_neon.h" "$host/arm_neon.h" >/dev/null 2>&1; then
		echo "darwin SDK clang headers already match the host toolchain"; exit 0
	fi
	[ -d "$tc/$ver/include.apple.bak" ] || mv "$tc/$ver/include" "$tc/$ver/include.apple.bak"
	rm -rf "$tc/$ver/include"
	cp -r "$host" "$tc/$ver/include"
	echo "replaced darwin SDK clang headers ($ver) with the host toolchain's"

# Catches unbalanced braces and malformed syntax — what a blind edit gets wrong.
ios-parse:
	#!/usr/bin/env bash
	set -euo pipefail
	mapfile -t files < <(find ios -name '*.swift' -not -path '*/build/*' -not -path '*/.build/*' -not -name 'Package.swift' | sort)
	[ "${#files[@]}" -gt 0 ] || { echo "error: no swift sources under ios/" >&2; exit 1; }
	swiftc -parse -swift-version 5 "${files[@]}"
	echo "parsed ${#files[@]} swift files"

# swift-format lints nothing here: its pretty printer wants the SwiftUI nesting
# this tree hand-packs, so it drives ios-fmt only (CI-010).
# Swift style per .swiftlint.yml; --strict, so a tuned rule's warning still fails.
ios-lint:
	#!/usr/bin/env bash
	set -euo pipefail
	# swiftlint dlopens sourcekitd and does not find it on its own here.
	lib="$(dirname "$(dirname "$(readlink -f "$(command -v swiftc)")")")/lib"
	[ -e "${lib}/libsourcekitdInProc.so" ] || { echo "error: sourcekitd not at ${lib}" >&2; exit 1; }
	LINUX_SOURCEKIT_LIB_PATH="${lib}" swiftlint lint --quiet --strict ios
	echo "swiftlint: clean"

# A whole-tree run reflows the packed SwiftUI closures, so pass a path.
# Rewrite Swift sources in swift-format's canonical style (.swift-format).
ios-fmt path=ios_dir:
	swift-format format --in-place --recursive --parallel {{path}}

fmt-check:
	cargo fmt --all --check

# ── Tests (test-*) ────────────────────────────────────────────────────────────

# Three tiers, split by source-tree location. Native package arguments come
# from Cargo metadata/default-members, so a new tested crate joins by existing.

# Everything under crates/. Excludes the browser tier, which is its own.
test-crates:
	#!/usr/bin/env bash
	set -euo pipefail
	package_output="$(ci/internal/runner/cargo-test-packages.sh crates)"
	mapfile -t packages <<<"${package_output}"
	[ "${#packages[@]}" -gt 0 ] && [ -n "${packages[0]:-}" ] \
	  || { echo "error: crates package inventory is empty" >&2; exit 1; }
	args=(); for package in "${packages[@]}"; do args+=(-p "${package}"); done
	cargo nextest run "${args[@]}"
	# The wire types again with the db impls compiled in: the default graph is
	# wasm-safe, so those `impl`s are only ever built here.
	cargo nextest run -p halogen-wire -p halogen-wire-meta --features halogen-wire/db

# A webui crate with a `mod tests` must be a default-member or it is silently
# absent here; `check-tests-run` is what enforces that.
# Everything under webui/ that runs natively.
test-webui:
	#!/usr/bin/env bash
	set -euo pipefail
	package_output="$(ci/internal/runner/cargo-test-packages.sh webui)"
	mapfile -t packages <<<"${package_output}"
	[ "${#packages[@]}" -gt 0 ] && [ -n "${packages[0]:-}" ] \
	  || { echo "error: webui package inventory is empty" >&2; exit 1; }
	args=(); for package in "${packages[@]}"; do args+=(-p "${package}"); done
	cargo nextest run "${args[@]}"

# Both native tiers, for `just test`-style muscle memory.
test: test-crates test-webui test-integ

# One test by nextest substring filter, e.g. `just test-one restart`.
test-one filter:
	cargo nextest run {{filter}}

# Documentation tests run through Cargo because nextest does not execute them.
test-doc:
	cargo test --doc

# The CI-script test tier: the behavioral guards under ci/internal/tests
# (release verdicts, reconciliation, stamps, evidence sanitization) plus
# syntax gates. Routine script edits: see the ci-script-checks skill.
test-apple-release:
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s ci/internal/tests -v
	node --test ci/internal/tests/test_*.js ci/internal/tests/test_*.mjs
	bash -n ci/internal/mac/decode-apple-profile.sh ci/internal/mac/enumerate-ios-tests.sh \
	  ci/internal/mac/ios-simulator.sh \
	  ci/internal/mac/ios-test-state.sh ci/internal/mac/ios-uniffi-bindgen.sh \
	  ci/internal/mac/validate-ios-xcresult.sh
	bash -n ci/internal/signing/sign-ios.sh
	# The Linux device lane (FEAT-048) — blind-edited most often, so at least
	# its syntax is gated here.
	bash -n ci/internal/device/apple-cc-env.sh ci/internal/device/build-local-xctest.sh \
	  ci/internal/signing/decode-apple-profile-linux.sh ci/internal/device/fetch-device-artifact.sh \
	  ci/internal/device/prepare-device-bundle.sh ci/internal/device/sign-ios-dev.sh
	python3 -m py_compile ci/internal/signing/validate_apple_bundle.py ci/internal/device/validate-device-results.py
	bash -n ci/internal/lib/install-if-changed.sh
	python3 -m py_compile ci/internal/mac/collect-ios-screenshots.py
	bash -n ci/internal/runner/ci-summary.sh
	node --check ci/internal/report/ci-report.mjs
	node --check ci/internal/release/forgejo-release.mjs
	node --check ci/internal/lib/forgejo-persistent-issue.mjs
	node --check ci/internal/report/ios-ci-metrics.mjs
	node --check ci/internal/report/ios-performance-report.mjs
	node --check ci/internal/release/ios-release-graph.mjs
	node --check ci/internal/release/ios-release-notes.mjs
	node --check ci/internal/report/ios-release-report.mjs
	node --check ci/internal/mac/ios-test-inventory.mjs
	node --check ci/internal/mac/ios-warm-manifest.mjs
	node --check ci/internal/mac/validate-ios-xcresult.mjs
	node ci/internal/mac/ios-test-inventory.mjs check ios/TestInventory.json
	node ci/internal/mac/ios-test-inventory.mjs check-sources \
	  ios/TestInventory.json ios/Tests ios/UITests
	jq empty ios/Halogen/Assets.xcassets/AppIcon.appiconset/Contents.json

test-e2e: ui-build
	ci/internal/runner/run-webui-e2e.sh

# ── Web UI (ui-*) ─────────────────────────────────────────────────────────────
# One-time setup: rustup target add wasm32-unknown-unknown

# Fails fast when the installed wasm-bindgen CLI mismatches Cargo.lock.
_ui-check-wbg:
	#!/usr/bin/env bash
	set -euo pipefail
	want="$(awk '/^name = "wasm-bindgen"$/{f=1} f && /^version = /{gsub(/[",]/,"",$3); print $3; exit}' Cargo.lock)"
	have="$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')" || true
	if [ "$have" != "$want" ]; then
	  echo "error: wasm-bindgen CLI ${have:-missing} != Cargo.lock $want" >&2
	  echo "fix: cargo install wasm-bindgen-cli --version $want" >&2
	  exit 1
	fi

# Compile Tailwind (v4 CLI) into the bundle stylesheet.
tailwind: _npm-install
	cd "{{uidir}}" && tailwindcss -i tailwind.css -o assets/tailwind.css --minify

# Build the Dioxus UI into dist/ (wasm + bindgen + static files + PWA shell).
ui-build: _ui-check-wbg tailwind
	#!/usr/bin/env bash
	set -euo pipefail
	cargo build -p halogen-webui --profile wasm-release --target wasm32-unknown-unknown
	mkdir -p "{{dist}}"
	find "{{dist}}" -mindepth 1 ! -name .gitkeep -delete
	wasm-bindgen --target web --no-typescript --out-dir "{{dist}}" --out-name webui \
	  "{{cargo_target}}/wasm32-unknown-unknown/wasm-release/halogen-ui.wasm"
	just _worker-build "{{dist}}"
	cp "{{uidir}}/index.html" "{{uidir}}/assets/tailwind.css" "{{dist}}/"
	cp "{{uidir}}/pwa/manifest.webmanifest" "{{uidir}}/pwa/worker.js" "{{dist}}/"
	cp "{{assets}}/favicon.ico" "{{dist}}/"
	mkdir -p "{{dist}}/icons"
	cp "{{assets}}/icon.svg" "{{assets}}/icon-192.png" "{{assets}}/icon-512.png" \
	  "{{assets}}/icon-maskable-192.png" "{{assets}}/icon-maskable-512.png" \
	  "{{assets}}/apple-touch-icon.png" "{{dist}}/icons/"
	# Stamp the service worker only after every app and worker asset is staged.
	version="$(awk -F '"' '/^version = /{print $2; exit}' "{{justfile_directory()}}/Cargo.toml")"
	cd "{{dist}}"
	files="$(find . -type f ! -name .gitkeep | sed 's|^./||' | sort)"
	hash="$(echo "$files" | xargs sha256sum | sha256sum | cut -c1-12)"
	precache="$(echo "$files" | awk '{printf "\"/%s\",", $0}' | sed 's/,$//')"
	sed -e "s/__CACHE_VERSION__/${version}-${hash}/" -e "s|__PRECACHE__|[${precache}]|" \
	  "{{uidir}}/pwa/sw.js" > sw.js

# Type-check the wasm UI crate (web-sys and dioxus move fast).
ui-check:
	cargo check -p halogen-webui --target wasm32-unknown-unknown

# ── iOS app (ios-*) ───────────────────────────────────────────────────────────
# One-time setup: rustup target add aarch64-apple-ios aarch64-apple-ios-sim,
# plus `xcodegen` and `typeshare-cli` on PATH. Pipeline: cargo staticlib →
# uniffi Swift bindings + typeshare wire types → xcodegen → xcodebuild.

ios_dir := justfile_directory() / "ios"
# Writes a generated file only when its content changed, so unchanged outputs
# keep their mtime (see the script for why that matters to Xcode).
# Publishes by hard link + rename, so a warm staticlib install is O(1).
install_if_changed := justfile_directory() / "ci" / "internal" / "lib" / "install-if-changed.sh"
# Fixed-label monotonic timings for warm iOS build substeps.
ios_build_timing := justfile_directory() / "ci" / "internal" / "report" / "ios-build-timing.mjs"
# Extra `xcodebuild` build settings, appended verbatim (e.g. "FOO=YES BAR=NO").
# Nothing is hardcoded here on purpose: run `just ios-cache-probe` on the build
# host to confirm a setting exists before putting it in this variable.
xcode_settings := env("HALOGEN_XCODE_SETTINGS", "")

# Swift Codable mirrors of the #[typeshare] wire DTOs.
ios-wire-types:
	node "{{ios_build_timing}}" typeshare -- just _ios-wire-types-impl

_ios-wire-types-impl:
	#!/usr/bin/env bash
	set -euo pipefail
	tmp="$(mktemp -d)"
	trap 'rm -rf "$tmp"' EXIT
	typeshare --lang=swift --config-file typeshare.toml --output-file "$tmp/WireTypes.swift" crates/wire crates/wire-meta
	'{{install_if_changed}}' "$tmp/WireTypes.swift" "{{ios_dir}}/Generated/WireTypes.swift"

# uniffi-bindgen reads metadata from the host cdylib, so the host build runs first.
# Simulator Rust core + regenerated Swift bindings.
ios-core: _ios-bindings
	#!/usr/bin/env bash
	set -euo pipefail
	# Only the .a is consumed for device/sim slices; --crate-type staticlib
	# skips the cdylib link, which Linux cannot drive for Mach-O anyway.
	IPHONEOS_DEPLOYMENT_TARGET=17.0 node "{{ios_build_timing}}" cargo-simulator -- \
	  ci/internal/device/apple-cc-env.sh iphonesimulator \
	  cargo rustc --timings -p halogen-mobile-ffi --target aarch64-apple-ios-sim --crate-type staticlib
	node "{{ios_build_timing}}" staticlib-install -- \
	  '{{install_if_changed}}' "{{cargo_target}}/aarch64-apple-ios-sim/debug/libhalogen_mobile.a" "{{ios_dir}}/Rust/lib/iphonesimulator/libhalogen_mobile.a"

# Xcode picks a slice by $(PLATFORM_NAME), so this composes with ios-core.
# Device slice of the Rust core (real hardware; needs signing).
ios-core-device: _ios-bindings
	IPHONEOS_DEPLOYMENT_TARGET=17.0 ci/internal/device/apple-cc-env.sh iphoneos \
	  cargo rustc --timings -p halogen-mobile-ffi --target aarch64-apple-ios --crate-type staticlib
	'{{install_if_changed}}' "{{cargo_target}}/aarch64-apple-ios/debug/libhalogen_mobile.a" "{{ios_dir}}/Rust/lib/iphoneos/libhalogen_mobile.a"

# The .pc SwiftPM's halogen_mobileFFI systemLibrary reads (FEAT-048). Absolute
# paths, so it lands in gitignored ios/Rust/, regenerated per checkout.
_ios-ffi-pc:
	#!/usr/bin/env bash
	set -euo pipefail
	mkdir -p '{{ios_dir}}/Rust'
	printf 'Name: halogen_mobile\nDescription: Halogen Rust core staticlib\nVersion: 0\nCflags: -I{{ios_dir}}/Rust/include\nLibs: -L{{ios_dir}}/Rust/lib/iphoneos -lhalogen_mobile\n' \
	  > '{{ios_dir}}/Rust/halogen_mobile.pc'

# FEAT-048 tier-0 gate: full typecheck (SwiftUI included) against the Darwin
# Swift SDK — needs Swift >= 6.3 (the toolchain image gates this) and a one-time
# `xtool sdk install` on this machine/container. HALOGEN_SWIFT overrides the binary.
swift_63 := env("HALOGEN_SWIFT", "swift")
ios-check: _ios-bindings _ios-ffi-pc
	# -j cap: SwiftPM defaults to nproc frontends and SwiftUI typechecks are
	# memory-hungry (~1GB each) — uncapped they can swap-thrash the host.
	cd '{{ios_dir}}' && PKG_CONFIG_PATH='{{ios_dir}}/Rust' '{{swift_63}}' build --swift-sdk arm64-apple-ios -j "${HALOGEN_SWIFT_JOBS:-4}"

# Mount the developer disk image on the dev phone (needed after every device
# reboot). Images live under data/run/ios-device/images.
ios-device-ddi:
	ios image auto --basedir=data/run/ios-device/images --udid="$DEV_DEVICE_UDID"

# FEAT-048 tier-1: pull the newest device-build artifact from CI into
# ios/build/device-testing.zip (see ci-report issue). Needs FORGEJO_TOKEN.
ios-device-fetch:
	ci/internal/device/fetch-device-artifact.sh ios/build/device-testing.zip

# FEAT-048 tier-1: install the CI device-testing.zip on the dev phone (signed
# with the dev identity) and launch the app. ZIP defaults to the newest fetched
# artifact; fetch it with `just ios-device-fetch`. Needs DEV_DEVICE_* env and a
# running go-ios userspace tunnel.
ios-device-run ZIP="ios/build/device-testing.zip":
	#!/usr/bin/env bash
	set -euo pipefail
	work="$(mktemp -d)"
	trap 'rm -rf "$work"' EXIT
	# Two-step so a prepare failure aborts here (eval "$(...)" masks the status).
	ids="$(ci/internal/device/prepare-device-bundle.sh {{quote(ZIP)}} "$work")"
	eval "$ids"
	ios launch --udid="$DEV_DEVICE_UDID" "$app_id"
	echo "launched $app_id on $DEV_DEVICE_UDID"

# Run device XCTest via go-ios. FILTER is Class or Class/method, with no module prefix; empty runs all UI tests.
# HALOGEN_SERVER_URL points to the published LAN fixture server used by device journeys.
ios-device-e2e FILTER="" ZIP="ios/build/device-testing.zip":
	#!/usr/bin/env bash
	set -euo pipefail
	case {{quote(FILTER)}} in *=*) echo "error: FILTER is positional, e.g. just <recipe> Class/test" >&2; exit 2;; esac
	work="$(mktemp -d)"
	trap 'rm -rf "$work"' EXIT
	ids="$(ci/internal/device/prepare-device-bundle.sh {{quote(ZIP)}} "$work")"
	eval "$ids"
	ci/internal/device/run-device-xctest.sh "$app_id" "$runner_id" {{quote(FILTER)}}

# FEAT-048 tier-1.5: cross-build ios/UITests into a device .xctest on Linux
# (ios/UITestsPkg harness; ~8s warm) -> ios/build/local/HalogenUITests.xctest.
ios-local-test-build:
	ci/internal/device/build-local-xctest.sh

# FEAT-048 tier-1.5: like ios-device-e2e, but runs the locally built .xctest
# swapped into the CI runner shell — no CI round trip for Swift test edits.
ios-device-e2e-local FILTER="" ZIP="ios/build/device-testing.zip": ios-local-test-build
	#!/usr/bin/env bash
	set -euo pipefail
	case {{quote(FILTER)}} in *=*) echo "error: FILTER is positional, e.g. just <recipe> Class/test" >&2; exit 2;; esac
	work="$(mktemp -d)"
	trap 'rm -rf "$work"' EXIT
	ids="$(ci/internal/device/prepare-device-bundle.sh {{quote(ZIP)}} "$work" ios/build/local/HalogenUITests.xctest)"
	eval "$ids"
	ci/internal/device/run-device-xctest.sh "$app_id" "$runner_id" {{quote(FILTER)}}

# FEAT-048 tier-1.5: build the dev app on Linux (xtool pack over the SwiftPM
# lane; ~40s warm). No Assets.car, so the springboard icon may be a placeholder.
# FFI API changes need `just ios-core` first to refresh the uniffi bindings.
ios-local-app-build: ios-core-device _ios-ffi-pc
	ci/internal/device/check-devinfo-parity.sh
	cd '{{ios_dir}}' && PKG_CONFIG_PATH='{{ios_dir}}/Rust' xtool dev build
	python3 ci/internal/device/stamp-local-app.py '{{ios_dir}}/xtool/Halogen.app'
	@echo "built {{ios_dir}}/xtool/Halogen.app"

# FEAT-048 tier-1.5: sign the locally built app with the dev identity, install
# it on the dev phone, and launch it. LAUNCH="" skips the launch.
ios-device-app-local LAUNCH="1": ios-local-app-build
	#!/usr/bin/env bash
	set -euo pipefail
	mkdir -p ios/build/local
	rm -rf ios/build/local/Halogen.app
	cp -a '{{ios_dir}}/xtool/Halogen.app' ios/build/local/Halogen.app
	ci/internal/device/sign-ios-dev.sh ios/build/local/Halogen.app
	ios install --udid="$DEV_DEVICE_UDID" --path=ios/build/local/Halogen.app
	[ -z {{quote(LAUNCH)}} ] || ios launch --udid="$DEV_DEVICE_UDID" --kill-existing org.fgsec.halogen.dev

# FEAT-048 all-local device test: build the app and the UI tests on Linux,
# install both, and run FILTER on the dev phone — the preferred way to test iOS
# when a device is attached (the CI zip supplies only the frozen runner shell).
ios-device-test FILTER="" ZIP="ios/build/device-testing.zip": ios-local-app-build ios-local-test-build
	#!/usr/bin/env bash
	set -euo pipefail
	case {{quote(FILTER)}} in *=*) echo "error: FILTER is positional, e.g. just <recipe> Class/test" >&2; exit 2;; esac
	work="$(mktemp -d)"
	trap 'rm -rf "$work"' EXIT
	ids="$(ci/internal/device/prepare-device-bundle.sh {{quote(ZIP)}} "$work" \
	  ios/build/local/HalogenUITests.xctest '{{ios_dir}}/xtool/Halogen.app')"
	eval "$ids"
	ci/internal/device/run-device-xctest.sh "$app_id" "$runner_id" {{quote(FILTER)}}

# xcodegen rewrites Halogen.xcodeproj and the generated Halogen/Info.plist, so it
# runs only when its inputs changed: project.yml plus the set of source PATHS it
# globs. Its own outputs are excluded or the stamp could never match.
# HALOGEN_XCODEGEN_FORCE=1 (or deleting the stamp) forces a regeneration.
_ios-project:
	node "{{ios_build_timing}}" xcodegen -- just _ios-project-impl

_ios-project-impl:
	#!/usr/bin/env bash
	set -euo pipefail
	cd '{{ios_dir}}'
	mkdir -p Generated .cache
	stamp=.cache/xcodegen-stamp
	want="$( { shasum -a 256 project.yml; \
	  find Halogen UITests Tests Generated -print 2>/dev/null || :; } \
	  | sed '/^Halogen\/Info\.plist$/d' | LC_ALL=C sort | shasum -a 256 | cut -d' ' -f1)"
	if [ -f Halogen.xcodeproj/project.pbxproj ] && [ -z "${HALOGEN_XCODEGEN_FORCE:-}" ] \
	   && [ "$(cat "$stamp" 2>/dev/null || true)" = "$want" ]; then
	  echo "Halogen.xcodeproj is up to date"
	else
	  xcodegen generate
	  printf '%s\n' "$want" > "$stamp"
	fi

# Build for the simulator. DEVICE empty = generic destination (local default);
# CI passes a concrete one so the cache keys match what `ios-e2e` builds.
ios-build DEVICE="": ios-core _ios-project
	#!/usr/bin/env bash
	set -euo pipefail
	device={{quote(DEVICE)}}
	destination='generic/platform=iOS Simulator'
	if [ -n "$device" ]; then
	  ci/internal/mac/ios-test-state.sh validate-device-name "$device"
	  simulator_id="$(node "{{ios_build_timing}}" destination-resolve -- \
	    ci/internal/mac/ios-simulator.sh resolve "$device")"
	  destination="platform=iOS Simulator,id=$simulator_id"
	fi
	node "{{ios_build_timing}}" xcodebuild -- \
	  xcodebuild -project "{{ios_dir}}/Halogen.xcodeproj" -scheme Halogen -configuration Debug \
	  -destination "$destination" \
	  -derivedDataPath "{{ios_dir}}/build" -showBuildTimingSummary \
	  CODE_SIGNING_ALLOWED=NO {{xcode_settings}} build

# Build, install, and launch in a simulator (first available iPhone by default).
ios-run DEVICE="": (ios-build DEVICE)
	#!/usr/bin/env bash
	set -euo pipefail
	device={{quote(DEVICE)}}
	if [ -z "$device" ]; then
	  device="$(ci/internal/mac/ios-simulator.sh resolve-default-name)"
	fi
	simulator_id="$(ci/internal/mac/ios-simulator.sh resolve "$device")"
	ci/internal/mac/ios-simulator.sh start-resolved "$device" "$simulator_id"
	ci/internal/mac/ios-simulator.sh wait-resolved "$device" "$simulator_id"
	echo "Using simulator: $device"
	open "$(xcode-select -p)/Applications/Simulator.app" 2>/dev/null || true
	xcrun simctl install "$simulator_id" "{{ios_dir}}/build/Build/Products/Debug-iphonesimulator/Halogen.app"
	xcrun simctl launch --console "$simulator_id" org.fgsec.halogen

# Routine functional UI lane. It builds the current revision once, then runs
# every functional journey serially; unit and screenshot coverage have their
# own lanes. Port override: HALOGEN_E2E_PORT=8792 just ios-e2e.
ios-e2e DEVICE="iPhone 17 Pro": (ios-build-for-testing DEVICE) ios-e2e-server (_ios-e2e-test-without-building DEVICE "e2e" "" "functional")

# One UI class or test, with the same exact-current build validation. A failed
# filtered run may use `ios-e2e-retry-one` in the same guest without rebuilding.
ios-e2e-one FILTER DEVICE="iPhone 17 Pro": (ios-build-for-testing DEVICE) ios-e2e-server (_ios-e2e-test-without-building DEVICE "e2e-one" FILTER "filtered")

ios-e2e-retry-one FILTER DEVICE="iPhone 17 Pro": (_ios-e2e-test-without-building DEVICE "e2e-retry" FILTER "filtered")

# Fast app-hosted unit lane: no UI test bundle and no loopback server.
test-ios-unit DEVICE="iPhone 17 Pro": (ios-unit-build-for-testing DEVICE) (ios-unit-test-without-building DEVICE)

# Protected/release coverage gate: one full build and one serial Xcode test
# session for the exact unit + functional + screenshot inventory.
test-ios-gate DEVICE="iPhone 17 Pro": (ios-build-for-testing DEVICE) ios-e2e-server (_ios-e2e-test-without-building DEVICE "gate" "" "gate") (_ios-export-screenshots "gate")

# One reviewed parity walkthrough (ios/ScreenshotInventory.json) → design/screenshots/ios/.
# The exact manifest is staged and replaces the committed tree only on success.
ios-screenshots DEVICE="iPhone 17 Pro": (ios-build-for-testing DEVICE) ios-e2e-server (_ios-e2e-test-without-building DEVICE "screenshots" "HalogenUITests/ScreenshotTests" "screenshots") (_ios-export-screenshots "screenshots")

_ios-export-screenshots BUNDLE:
	#!/usr/bin/env bash
	set -euo pipefail
	bundle={{quote(BUNDLE)}}
	ci/internal/mac/ios-test-state.sh validate-bundle "$bundle"
	out="{{justfile_directory()}}/design/screenshots/ios"
	# Captured aside and swapped in only on success, the shape web-screenshots
	# uses: deleting the committed tree before the export is known to have
	# produced anything replaces real parity evidence with nothing (IOS-TESTS).
	parent="$(dirname "$out")"
	mkdir -p "$parent"
	raw="$(mktemp -d)"
	staged="$(mktemp -d "${parent}/.ios-screenshots.stage.XXXXXX")"
	backup=""
	cleanup() {
	  rm -rf "$raw"
	  [ -z "$staged" ] || rm -rf "$staged"
	  if [ -n "$backup" ] && [ -e "$backup" ]; then
	    if [ ! -e "$out" ]; then mv "$backup" "$out"; else rm -rf "$backup"; fi
	  fi
	}
	trap cleanup EXIT
	rm -f "{{ios_dir}}/build/${bundle}-attachments-manifest.json"
	xcrun xcresulttool export attachments \
	  --path "{{ios_dir}}/build/${bundle}.xcresult" --output-path "$raw" >/dev/null
	cp "$raw/manifest.json" "{{ios_dir}}/build/${bundle}-attachments-manifest.json"
	python3 "{{justfile_directory()}}/ci/internal/mac/collect-ios-screenshots.py" "$raw" "$staged"
	python3 "{{justfile_directory()}}/ci/internal/mac/collect-ios-screenshots.py" \
	  --validate-product "$staged"
	if [ -e "$out" ]; then
	  backup="$(mktemp -d "${parent}/.ios-screenshots.previous.XXXXXX")"
	  rmdir "$backup"
	  mv "$out" "$backup"
	fi
	mv "$staged" "$out"
	staged=""
	if [ -n "$backup" ]; then rm -rf "$backup"; backup=""; fi
	chmod 755 "$out"   # mktemp -d is 0700; match screenshots/web

# Browser captures of every journey -> design/screenshots/web/ (committed:
# the web half of design/screenshots, next to the iOS captures).
# One subdirectory per journey, one PNG per named step. Needs chromium +
# chromedriver, same as `just test-e2e`.
web-screenshots:
	#!/usr/bin/env bash
	set -euo pipefail
	out="{{justfile_directory()}}/design/screenshots/web"
	# Captured aside and swapped in only on success: HALOGEN_E2E_REQUIRED turns
	# the browser tier's silent skip (no chromedriver) into the failure it is,
	# so a dev box without chromium cannot empty the committed tree.
	tmp="$(mktemp -d)"
	trap 'rm -rf "$tmp"' EXIT
	HALOGEN_E2E_REQUIRED=1 HALOGEN_E2E_SCREENSHOT_DIR="$tmp" just test-e2e
	[ -n "$(ls -A "$tmp")" ] || { echo "error: no screenshots were produced" >&2; exit 1; }
	rm -rf "$out"
	mkdir -p "$(dirname "$out")"
	mv "$tmp" "$out"
	chmod 755 "$out"   # mktemp -d is 0700; match screenshots/ios

# Fail early with a readable message: cargo's own error for an unresolved
# source replacement does not mention what to set (.env / .env.example).
_require-registry:
	#!/usr/bin/env bash
	[ -n "${CARGO_REGISTRIES_CHILLED_PROXY_INDEX:-}" ] || {
	  echo "error: CARGO_REGISTRIES_CHILLED_PROXY_INDEX is empty" >&2
	  echo "       set SERVICES_ROOT_DOMAIN in .env, or the index explicitly" >&2
	  exit 1
	}

# Refuse to run against a listener we did not start. A leaked server from an
# aborted run answers the readiness probe with a stale database, failing the
# suite for a reason that looks like a product bug.
_require-e2e-port:
	#!/usr/bin/env bash
	port="${HALOGEN_E2E_PORT:-8099}"
	if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
	  echo "error: port $port is already in use — refusing to test against it:" >&2
	  lsof -nP -iTCP:"$port" -sTCP:LISTEN >&2 || true
	  echo "       stop it, or set HALOGEN_E2E_PORT to a free port" >&2
	  exit 1
	fi

# Loopback server binary used by functional and combined-gate executions.
ios-e2e-server: _require-registry
	cargo build --timings -p halogen-server

# Execute a prebuilt UI bundle. Every maintained UI lane is serial.
_ios-e2e-test-without-building DEVICE BUNDLE ONLY MODE:
	#!/usr/bin/env bash
	set -euo pipefail
	cd "{{justfile_directory()}}"
	device={{quote(DEVICE)}}
	bundle={{quote(BUNDLE)}}
	only={{quote(ONLY)}}
	mode={{quote(MODE)}}
	ci/internal/mac/ios-test-state.sh validate-device-name "$device"
	ci/internal/mac/ios-test-state.sh validate-bundle "$bundle"
	case "${HALOGEN_IOS_RECORD_VIDEO:-}" in
	  ""|1) ;; *) echo "error: HALOGEN_IOS_RECORD_VIDEO must be empty or 1" >&2; exit 2;;
	esac
	selection=()
	need_server=1
	if [ "$mode" = functional ] && [ -z "$only" ]; then
	  selection=(-only-testing:HalogenUITests -skip-testing:HalogenUITests/ScreenshotTests)
	elif [ "$mode" = screenshots ] && [ "$only" = HalogenUITests/ScreenshotTests ]; then
	  selection=(-only-testing:HalogenUITests/ScreenshotTests)
	  need_server=1
	elif [ "$mode" = filtered ] && [ -n "$only" ]; then
	  filter="$only"
	  ci/internal/mac/ios-test-state.sh validate-filter "$filter"
	  case "$filter" in HalogenUITests/*) ;; *) filter="HalogenUITests/$filter" ;; esac
	  selection=(-only-testing:"$filter")
	elif [ "$mode" = gate ] && [ -z "$only" ]; then
	  selection=()
	else
	  echo "error: invalid iOS test mode/filter" >&2; exit 2
	fi
	simulator_id="$(ci/internal/mac/ios-test-state.sh bound-id '{{ios_dir}}/build' Halogen "$device")"
	video_path="{{ios_dir}}/build/${bundle}.mov"
	result_path="{{ios_dir}}/build/${bundle}.xcresult"
	rm -f "$video_path"
	root=""
	server_pid=""
	vid_pid=""
	cleanup() {
	  if [ -n "$vid_pid" ]; then
	    kill -INT "$vid_pid" 2>/dev/null || true
	    if wait "$vid_pid" 2>/dev/null; then video_status=0; else video_status=$?; fi
	    if [ "$video_status" -ne 0 ] || [ ! -s "$video_path" ]; then rm -f "$video_path"; fi
	  fi
	  [ -z "$server_pid" ] || kill "$server_pid" 2>/dev/null || true
	  [ -z "$server_pid" ] || wait "$server_pid" 2>/dev/null || true
	  [ -z "$root" ] || rm -rf "$root"
	}
	trap cleanup EXIT
	if [ -n "$need_server" ]; then
	  just _require-e2e-port
	  [ -x "{{cargo_target}}/debug/halogen-server" ] \
	    || { echo "error: run just ios-e2e-server before executing UI tests" >&2; exit 1; }
	  port="${HALOGEN_E2E_PORT:-8099}"
	  root="$(mktemp -d)"
	  mkdir -p "$root/media" "$root/public/feeds"
	  python3 ci/internal/lib/seed-discover-fixtures.py "$root/public" "$port"
	  export HALOGEN_DISCOVER_ITUNES_BASE_URL="http://127.0.0.1:$port/discover/itunes.json"
	  export HALOGEN_DISCOVER_GPODDER_BASE_URL="http://127.0.0.1:$port/discover/gpodder.json"
	  server_binary="$(python3 -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).resolve())' '{{cargo_target}}/debug/halogen-server')"
	  (cd "$root"; exec "$server_binary" \
	    --db-path "$root/server.db" --media-root "$root/media" \
	    --enable-public-server --public-root "$root/public" \
	    --admin-username dev --admin-password dev \
	    --auth-token-secret ios-e2e-loopback-secret-0123456789abcdef \
	    --discover-itunes-base-url "$HALOGEN_DISCOVER_ITUNES_BASE_URL" \
	    --discover-gpodder-base-url "$HALOGEN_DISCOVER_GPODDER_BASE_URL" \
	    --listen-port "$port" --allow-private-network --dev-use-mock-download --dev-seed-data) &
	  server_pid=$!
	  ready=""
	  deadline=$((SECONDS + 55))
	  while (( SECONDS < deadline )); do
	    kill -0 "$server_pid" 2>/dev/null \
	      || { echo "error: halogen-server exited during startup (see log above)" >&2; exit 1; }
	    if curl -fsS --connect-timeout 1 --max-time 1 \
	        "http://127.0.0.1:$port/healthz" >/dev/null 2>&1; then
	      ready=1; break
	    fi
	    sleep 0.2
	  done
	  [ -n "$ready" ] \
	    || { echo "error: halogen-server did not become ready on port $port" >&2; exit 1; }
	  export TEST_RUNNER_HALOGEN_E2E_BASE="http://127.0.0.1:$port"
	else
	  unset TEST_RUNNER_HALOGEN_E2E_BASE
	fi
	ci/internal/mac/ios-simulator.sh start-resolved "$device" "$simulator_id"
	ci/internal/mac/ios-simulator.sh wait-resolved "$device" "$simulator_id"
	[ -n "${HEADLESS:-}" ] || open "$(xcode-select -p)/Applications/Simulator.app" 2>/dev/null || true
	if [ "${HALOGEN_IOS_RECORD_VIDEO:-}" = 1 ]; then
	  xcrun simctl io "$simulator_id" recordVideo --force "$video_path" \
	    >/dev/null 2>&1 &
	  vid_pid=$!
	fi
	rm -rf "$result_path"
	expected="{{ios_dir}}/build/${bundle}.expected-tests.json"
	ci/internal/mac/enumerate-ios-tests.sh "$device" Halogen "$expected" "$mode" "$only"
	started=$SECONDS
	xcodebuild test-without-building \
	  -project "{{ios_dir}}/Halogen.xcodeproj" -scheme Halogen \
	  -destination "platform=iOS Simulator,id=$simulator_id" \
	  -derivedDataPath "{{ios_dir}}/build" \
	  -resultBundlePath "$result_path" \
	  -parallel-testing-enabled NO \
	  ${selection[@]+"${selection[@]}"} \
	  CODE_SIGNING_ALLOWED=NO {{xcode_settings}}
	echo "iOS UI test execution: $((SECONDS - started))s"
	ci/internal/mac/validate-ios-xcresult.sh "$result_path" "$mode" "$expected"

# Validate before expensive dependencies, but boot only after build/server work.
_ios-simulator-validate DEVICE:
	node "{{ios_build_timing}}" early-device-validation -- \
	  ci/internal/mac/ios-test-state.sh validate-device {{quote(DEVICE)}}

# App plus both test bundles without running them. The source fingerprint stamp
# prevents a later test-without-building from consuming yesterday's xctestrun.
ios-build-for-testing DEVICE="iPhone 17 Pro": (_ios-simulator-validate DEVICE) ios-core (_ios-build-for-testing-current-core DEVICE)

# Internal entry for a caller that has already built the exact current Rust
# core in this process. Public callers use ios-build-for-testing above.
_ios-build-for-testing-current-core DEVICE="iPhone 17 Pro": _ios-project
	#!/usr/bin/env bash
	set -euo pipefail
	device={{quote(DEVICE)}}
	ci/internal/mac/ios-test-state.sh validate-device-name "$device"
	simulator_id="$(node "{{ios_build_timing}}" destination-resolve -- \
	  ci/internal/mac/ios-simulator.sh resolve "$device")"
	started=$SECONDS
	node "{{ios_build_timing}}" xcodebuild -- xcodebuild build-for-testing \
	  -project "{{ios_dir}}/Halogen.xcodeproj" -scheme Halogen \
	  -destination "platform=iOS Simulator,id=$simulator_id" \
	  -derivedDataPath "{{ios_dir}}/build" \
	  -showBuildTimingSummary CODE_SIGNING_ALLOWED=NO {{xcode_settings}}
	node "{{ios_build_timing}}" mark -- \
	  ci/internal/mac/ios-test-state.sh mark '{{ios_dir}}/build' Halogen "$device" "$simulator_id"
	echo "iOS build-for-testing: $((SECONDS - started))s"

# FEAT-048: device-arch app + UI test bundles for the Linux device loop.
# Unsigned (the sandbox signs with the dev identity and runs via go-ios);
# generic destination, so no simulator is resolved. Produces one zip of the
# Debug-iphoneos products plus the xctestrun.
ios-device-build-for-testing: ios-core ios-core-device _ios-project
	#!/usr/bin/env bash
	set -euo pipefail
	started=$SECONDS
	node "{{ios_build_timing}}" xcodebuild -- xcodebuild build-for-testing \
	  -project "{{ios_dir}}/Halogen.xcodeproj" -scheme Halogen \
	  -destination "generic/platform=iOS" \
	  -derivedDataPath "{{ios_dir}}/build" \
	  -showBuildTimingSummary CODE_SIGNING_ALLOWED=NO {{xcode_settings}}
	out="{{ios_dir}}/build/device-testing"
	rm -rf "$out" "{{ios_dir}}/build/device-testing.zip"
	mkdir -p "$out"
	cp -R "{{ios_dir}}/build/Build/Products/Debug-iphoneos" "$out/Products"
	cp "{{ios_dir}}/build/Build/Products/"*_iphoneos*.xctestrun "$out/"
	(cd "$out" && zip -qry "{{ios_dir}}/build/device-testing.zip" .)
	echo "iOS device build-for-testing: $((SECONDS - started))s -> ios/build/device-testing.zip"

ios-unit-build-for-testing DEVICE="iPhone 17 Pro": (_ios-simulator-validate DEVICE) ios-core (_ios-unit-build-for-testing-current-core DEVICE)

_ios-unit-build-for-testing-current-core DEVICE="iPhone 17 Pro": _ios-project
	#!/usr/bin/env bash
	set -euo pipefail
	device={{quote(DEVICE)}}
	ci/internal/mac/ios-test-state.sh validate-device-name "$device"
	simulator_id="$(node "{{ios_build_timing}}" destination-resolve -- \
	  ci/internal/mac/ios-simulator.sh resolve "$device")"
	started=$SECONDS
	node "{{ios_build_timing}}" xcodebuild -- xcodebuild build-for-testing \
	  -project "{{ios_dir}}/Halogen.xcodeproj" -scheme HalogenUnit \
	  -destination "platform=iOS Simulator,id=$simulator_id" \
	  -derivedDataPath "{{ios_dir}}/build" \
	  -showBuildTimingSummary CODE_SIGNING_ALLOWED=NO {{xcode_settings}}
	node "{{ios_build_timing}}" mark -- \
	  ci/internal/mac/ios-test-state.sh mark '{{ios_dir}}/build' HalogenUnit "$device" "$simulator_id"
	echo "iOS unit build-for-testing: $((SECONDS - started))s"

ios-unit-test-without-building DEVICE="iPhone 17 Pro": (_ios-unit-test-without-building DEVICE "unit")

_ios-unit-test-without-building DEVICE BUNDLE:
	#!/usr/bin/env bash
	set -euo pipefail
	bundle={{quote(BUNDLE)}}
	device={{quote(DEVICE)}}
	ci/internal/mac/ios-test-state.sh validate-device-name "$device"
	ci/internal/mac/ios-test-state.sh validate-bundle "$bundle"
	simulator_id="$(ci/internal/mac/ios-test-state.sh bound-id '{{ios_dir}}/build' HalogenUnit "$device")"
	result_path="{{ios_dir}}/build/${bundle}.xcresult"
	rm -rf "$result_path"
	expected="{{ios_dir}}/build/${bundle}.expected-tests.json"
	ci/internal/mac/enumerate-ios-tests.sh "$device" HalogenUnit "$expected" unit
	ci/internal/mac/ios-simulator.sh start-resolved "$device" "$simulator_id"
	ci/internal/mac/ios-simulator.sh wait-resolved "$device" "$simulator_id"
	started=$SECONDS
	xcodebuild test-without-building \
	  -project "{{ios_dir}}/Halogen.xcodeproj" -scheme HalogenUnit \
	  -destination "platform=iOS Simulator,id=$simulator_id" \
	  -derivedDataPath "{{ios_dir}}/build" \
	  -resultBundlePath "$result_path" \
	  -parallel-testing-enabled NO -only-testing:HalogenTests \
	  CODE_SIGNING_ALLOWED=NO {{xcode_settings}}
	echo "iOS unit test execution: $((SECONDS - started))s"
	ci/internal/mac/validate-ios-xcresult.sh "$result_path" unit "$expected"

# Use unstripped host metadata for bindings; optimize the device and simulator cores.
ios-core-release: _ios-bindings
	IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build --timings -p halogen-mobile-ffi --release --target aarch64-apple-ios-sim
	IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build --timings -p halogen-mobile-ffi --release --target aarch64-apple-ios
	'{{install_if_changed}}' "{{cargo_target}}/aarch64-apple-ios-sim/release/libhalogen_mobile.a" "{{ios_dir}}/Rust/lib/iphonesimulator/libhalogen_mobile.a"
	'{{install_if_changed}}' "{{cargo_target}}/aarch64-apple-ios/release/libhalogen_mobile.a" "{{ios_dir}}/Rust/lib/iphoneos/libhalogen_mobile.a"

# Unsigned Release archive; signing keys remain on the Mac host.
ios-archive: ios-core-release _ios-archive-current-core

# Internal entry for measured CI after ios-core-release has already succeeded.
_ios-archive-current-core: _ios-project
	#!/usr/bin/env bash
	set -euo pipefail
	out="{{justfile_directory()}}/target/ios-release"
	mkdir -p "$out"
	rm -rf "$out/Halogen.xcarchive"
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' "{{justfile_directory()}}/Cargo.toml" | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -n1)"
	[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "error: iOS releases require an App Store version like 1.2.3" >&2; exit 1; }
	# Three period-separated components, each well under 2^32: a flat
	# timestamp overflows what App Store Connect accepts per component.
	# Monotonic across days, months, and years (0811 < 0901 < 2027's 0101).
	build="$(date -u +%Y.%m%d.%H%M)"
	xcodebuild -project "{{ios_dir}}/Halogen.xcodeproj" -scheme Halogen -configuration Release \
	  -destination 'generic/platform=iOS' -archivePath "$out/Halogen.xcarchive" \
	  -derivedDataPath "$out/build" -showBuildTimingSummary \
	  MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$build" \
	  CODE_SIGNING_ALLOWED=NO {{xcode_settings}} archive

# Stage the unsigned app for host-side validation and signing. The dSYMs come
# with it: the archive dies with the ephemeral VM, and a rebuild produces new
# UUIDs, so symbols not collected here can never symbolicate this build.
ios-collect: ios-archive _ios-collect-current-archive

_ios-collect-current-archive:
	#!/usr/bin/env bash
	set -euo pipefail
	archive="{{justfile_directory()}}/target/ios-release/Halogen.xcarchive"
	out="{{justfile_directory()}}/artifacts/release/ios"
	rm -rf "$out"
	mkdir -p "$out"
	cp -R "$archive/Products/Applications/Halogen.app" "$out/"
	[ -d "$archive/dSYMs" ] || { echo "error: archive has no dSYMs" >&2; exit 1; }
	cp -R "$archive/dSYMs" "$out/"

# Does this Xcode define the Xcode 26 compilation-caching build settings? Three
# checks, none of which passes a candidate name in — `-showBuildSettings` echoes
# command-line overrides, so asking it about a name we supplied proves nothing.
ios-cache-probe:
	#!/usr/bin/env bash
	set -euo pipefail
	dev="$(xcode-select -p)"
	echo "== 3. ground truth: identifiers the Xcode bundle itself defines"
	echo "   (greps several GB of the Xcode bundle — expect minutes, no progress)"
	found=""
	for root in "$dev/../SharedFrameworks" "$dev/Library" "$dev/../PlugIns"; do
	  [ -d "$root" ] || continue
	  hit="$(grep -rhoas -E 'SWIFT_ENABLE_[A-Z_]+|CLANG_ENABLE_[A-Z_]+|[A-Z_]*CACH[A-Z_]*' "$root" | LC_ALL=C sort -u || true)"
	  if [ -n "$hit" ]; then found="$found$hit"$'\n'; fi
	done
	if [ -n "$found" ]; then printf '%s' "$found" | LC_ALL=C sort -u; else echo "(no such identifier in the searched Xcode directories)"; fi
	if [ ! -f '{{ios_dir}}/Halogen.xcodeproj/project.pbxproj' ]; then
	  echo; echo "no Halogen.xcodeproj yet — run 'just ios-build' for checks 1 and 2"; exit 0
	fi
	cd '{{ios_dir}}'
	echo; echo "== 1. settings this Xcode defines by default (no overrides passed)"
	xcodebuild -project Halogen.xcodeproj -scheme Halogen -configuration Debug -showBuildSettings 2>/dev/null \
	  | grep -iE 'COMPILATION_CACHE|COMPILE_CACHE|EXPLICIT_MODULES|PREFIX_MAPPING' || echo "(none)"
	echo; echo "== 2. negative control: does an invented name echo back?"
	if xcodebuild -project Halogen.xcodeproj -scheme Halogen -configuration Debug -showBuildSettings \
	     HALOGEN_NOT_A_REAL_SETTING=YES 2>/dev/null | grep -q HALOGEN_NOT_A_REAL_SETTING; then
	  echo "FAIL: -showBuildSettings echoes unknown names, so check 1 proves nothing — trust check 3 only"
	else
	  echo "ok: unknown names do not appear, so check 1 is meaningful"
	fi
	echo
	echo "Set HALOGEN_XCODE_SETTINGS only for names check 3 found (and check 2 said ok),"
	echo "then confirm ios/build/CompilationCache.noindex appears and grows."

# ── CI build directory (ci-*workdir*) ─────────────────────────────────────────
# The runner checks out to a new absolute path every run, and both cargo (path
# source ids, mtime fingerprints) and Xcode (incremental state, module PCMs)
# key on absolute paths. HALOGEN_WORK_DIR gives CI one stable root instead.

# Mirror this checkout into HALOGEN_WORK_DIR. git, not a copy: `checkout --force`
# rewrites only files whose blob changed, so unchanged files keep their mtime.
# Unset HALOGEN_WORK_DIR and everything below builds in the checkout as before.
ci-sync-workdir:
	#!/usr/bin/env bash
	set -euo pipefail
	dest='{{work_dir}}'
	src='{{justfile_directory()}}'
	if [ -z "$dest" ]; then
	  echo "HALOGEN_WORK_DIR is unset — building in the checkout ($src)"
	  exit 0
	fi
	case "$dest" in /*) ;; *) echo "error: HALOGEN_WORK_DIR must be absolute" >&2; exit 1 ;; esac
	if [ "$dest" = "$src" ]; then echo "error: HALOGEN_WORK_DIR is the checkout" >&2; exit 1; fi
	# Matched against a value, not emptiness: an expression engine that renders
	# a false branch as the string "false" would otherwise reset every run.
	if [ "${HALOGEN_WORKDIR_RESET:-}" = 1 ]; then
	  echo "HALOGEN_WORKDIR_RESET=1 — discarding $dest and rebuilding cold"
	  rm -rf "$dest"
	fi
	if ! git -C "$src" rev-parse --git-dir > /dev/null 2>&1; then
	  echo "error: $src is not a git repository, so it cannot be mirrored" >&2; exit 1
	fi
	mkdir -p "$dest"
	if [ ! -d "$dest/.git" ]; then git -C "$dest" init -q; fi
	# By filesystem path with no named remote, so no credential can be persisted
	# here; gc off so no repack pause lands on the critical path.
	if ! git -C "$dest" -c gc.auto=0 fetch -q --no-tags --depth=1 --force "$src" HEAD; then
	  echo "note: shallow fetch refused; retrying at full depth"
	  git -C "$dest" -c gc.auto=0 fetch -q --no-tags --force "$src" HEAD
	fi
	if git -C "$dest" rev-parse --verify -q HEAD > /dev/null; then
	  echo "incoming: $(git -C "$dest" diff --shortstat HEAD FETCH_HEAD)"
	fi
	git -C "$dest" checkout -q --force --detach FETCH_HEAD
	git -C "$dest" clean -qfd   # no -x: the gitignored build outputs are the cache
	echo "work dir: $dest ($(git -C "$dest" rev-parse --short HEAD))"

# The work dir is baked into a retained image. The producer workflow owns this
# check and refuses anything secret-shaped before marking the image complete.
ci-assert-workdir-clean:
	#!/usr/bin/env bash
	set -euo pipefail
	dest='{{work_dir}}'
	if [ -z "$dest" ] || [ ! -d "$dest" ]; then echo "no work dir to check"; exit 0; fi
	if ! hit="$(find "$dest" -maxdepth 3 \( -name '.env' -o -name '*.p12' -o -name '*.p8' \
	  -o -name '*.mobileprovision' -o -name '*.keychain-db' \) -print -quit)"; then
	  echo "error: credential scan could not inspect the retained work dir" >&2
	  exit 1
	fi
	if [ -n "$hit" ]; then echo "error: $hit would be baked into the image" >&2; exit 1; fi
	if [ -d "$dest/.git" ] \
	   && git -C "$dest" config --local --get-regexp '^(http|remote)\.' > /dev/null 2>&1; then
	  echo "error: the work dir git config has a remote or http section" >&2; exit 1
	fi
	echo "work dir carries no credential material"

# ── Apple signing (apple-*) ───────────────────────────────────────────────────
# Distribution signing stays local, using a container or installed Linux tools.
# CI verifies the signed IPA and prepares its TestFlight asset description.

# Needs APPLE_SIGNING_IDENTITY, APPLE_P12_PATH, APPLE_P12_PASSWORD and
# IOS_PROVISIONING_PROFILE (.env), plus a Forgejo token. `no` skips the dispatch.
# TAG defaults to ios-v<workspace version>; that release must already exist.
# Sign a released iOS build here, attach the IPA, and dispatch its verification.
ios-sign-local tag="" verify="yes":
	#!/usr/bin/env bash
	set -euo pipefail
	tag='{{tag}}'
	# Default to the workspace version, the same extraction ci-tagged-release-ios
	# tags with — so signing follows the tree without repeating the version.
	if [ -z "${tag}" ]; then
	  version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml \
	    | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -n1)"
	  [ -n "${version}" ] \
	    || { echo "error: workspace version not found in Cargo.toml" >&2; exit 1; }
	  tag="ios-v${version}"
	  echo "no tag given; using ${tag} from the workspace version"
	fi
	[[ "${tag}" =~ ^ios-v[0-9]+\.[0-9]+\.[0-9]+$ ]] \
	  || { echo "error: tag must look like ios-v1.2.3" >&2; exit 2; }
	version="${tag#ios-v}"
	case "${HALOGEN_SIGNING_RUNTIME:-container}" in
	  native) [ "$(uname -s)" = Linux ] || { echo "error: native signing requires Linux" >&2; exit 2; } ;;
	  container) [ -n "{{toolchain_image}}" ] || { echo "error: set TOOLCHAIN_IMAGE or SERVICES_ROOT_DOMAIN" >&2; exit 1; } ;;
	  *) echo "error: HALOGEN_SIGNING_RUNTIME must be container or native" >&2; exit 2 ;;
	esac
	token="$(just _fj-token)"
	for var in APPLE_SIGNING_IDENTITY APPLE_P12_PATH APPLE_P12_PASSWORD IOS_PROVISIONING_PROFILE; do
	  [ -n "${!var:-}" ] || { echo "error: ${var} is required (see .env.example)" >&2; exit 1; }
	done
	[ -r "${APPLE_P12_PATH}" ] || { echo "error: cannot read ${APPLE_P12_PATH}" >&2; exit 1; }
	[ -r "${IOS_PROVISIONING_PROFILE}" ] \
	  || { echo "error: cannot read ${IOS_PROVISIONING_PROFILE}" >&2; exit 1; }
	slug="$(just _fj-slug)"
	api="https://${slug%%/*}/api/v1/repos/${slug#*/}"

	# Everything downloaded, signed, and uploaded lives here (git-ignored).
	work="{{signing_dir}}/${tag}"
	rm -rf "${work}"
	mkdir -p "${work}"
	chmod 700 "${work}"

	run_signer() {
	  HALOGEN_SIGNING_IMAGE="{{toolchain_image}}" HALOGEN_SIGNING_ENGINE="{{engine}}" \
	    ci/internal/signing/run-local.sh sign "$work" "$@"
	}
	run_signer --check-tools

	echo "==> resolving ${tag}"
	release_id="$(just _fj-api GET "releases/tags/${tag}" | jq -er '.id')"
	assets="$(just _fj-api GET "releases/${release_id}/assets")"
	fetch() {
	  url="$(printf '%s' "${assets}" \
	    | jq -er --arg n "$1" '.[] | select(.name == $n) | .browser_download_url')" \
	    || { echo "error: $1 is not attached to ${tag}" >&2; exit 1; }
	  curl -fsSL --max-time 900 -H "Authorization: token ${token}" -o "${work}/$2" "${url}"
	}
	echo "==> downloading the unsigned build"
	fetch "halogen-${version}-ios-unsigned.zip" unsigned.zip
	fetch "halogen-${version}-ios-unsigned.zip.sha256" unsigned.zip.sha256
	fetch "halogen-${version}-ios-dsyms.zip" dsyms.zip
	# What the build lane recorded must be what we just fetched.
	(cd "${work}" && awk '{print $1"  unsigned.zip"}' unsigned.zip.sha256 | sha256sum -c -)

	# Pass signing material through environment variables, never command arguments.
	echo "==> signing with ${HALOGEN_SIGNING_RUNTIME:-container} tools"
	APPLE_P12_BASE64="$(base64 -w0 < "${APPLE_P12_PATH}")"
	IOS_PROVISIONING_PROFILE_BASE64="$(base64 -w0 < "${IOS_PROVISIONING_PROFILE}")"
	export APPLE_P12_BASE64 IOS_PROVISIONING_PROFILE_BASE64
	run_signer unsigned.zip "${version}" dsyms.zip

	ipa="halogen-${version}-ios.ipa"
	[ -f "${work}/${ipa}" ] || { echo "error: ${ipa} was not produced" >&2; exit 1; }

	# The release is the only channel to the verifying runner, so the IPA is
	# attached before anything has judged it; a failed verification withdraws it.
	echo "==> attaching to release ${release_id}"
	attach() {
	  for id in $(just _fj-api GET "releases/${release_id}/assets" \
	                | jq -r --arg n "$1" '.[] | select(.name == $n) | .id'); do
	    just _fj-api DELETE "releases/${release_id}/assets/${id}" >/dev/null
	  done
	  curl -fsS --max-time 900 -X POST \
	    -H "Authorization: token ${token}" \
	    -F "attachment=@${work}/$1" \
	    "${api}/releases/${release_id}/assets?name=$1" >/dev/null
	  echo "attached $1"
	}
	attach "${ipa}"
	attach "${ipa}.sha256"

	if [ '{{verify}}' != yes ]; then
	  echo "skipped verification; run: just fj-dispatch ios-verify-signing.yml master '{\"tag\":\"${tag}\"}'"
	  exit 0
	fi
	just fj-dispatch ios-verify-signing.yml "${HALOGEN_VERIFY_REF:-master}" \
	  "$(jq -nc --arg t "${tag}" --arg i "${APPLE_SIGNING_IDENTITY}" '{tag: $t, identity: $i}')"
	echo "watch it with: just fj-runs ios-verify-signing.yml"

# Fetch Transporter once for local TestFlight uploads, using the selected signing runtime.
# Its license requires acceptance and forbids shared redistribution, so keep it git-ignored, outside images.
# INSTALLER selects a local package instead of downloading.
ios-transporter-setup installer="":
	#!/usr/bin/env bash
	set -euo pipefail
	dir='{{transporter_dir}}'
	case "${HALOGEN_SIGNING_RUNTIME:-container}" in
	  native) [ "$(uname -s)" = Linux ] || { echo "error: native signing requires Linux" >&2; exit 2; } ;;
	  container) [ -n "{{toolchain_image}}" ] || { echo "error: set TOOLCHAIN_IMAGE or SERVICES_ROOT_DOMAIN" >&2; exit 1; } ;;
	  *) echo "error: HALOGEN_SIGNING_RUNTIME must be container or native" >&2; exit 2 ;;
	esac
	scratch="$(mktemp -d)"
	trap 'rm -rf "${scratch}"' EXIT
	chmod 700 "${scratch}"

	if [ -n '{{installer}}' ]; then
	  [ -r '{{installer}}' ] || { echo "error: cannot read {{installer}}" >&2; exit 1; }
	  cp '{{installer}}' "${scratch}/itms.sh"
	else
	  # Apple answers GET but not HEAD here, and serves whatever is current;
	  # there is no versioned URL to pin to.
	  echo "==> downloading Transporter from Apple (~146MB)"
	  curl -fsSL --max-time 1800 -o "${scratch}/itms.sh" \
	    'https://itunesconnect.apple.com/WebObjects/iTunesConnect.woa/ra/resources/download/public/Transporter__Linux/bin'
	fi

	run_installer() {
	  if [ "${HALOGEN_SIGNING_RUNTIME:-container}" = native ]; then
	    (cd "$scratch" && sh ./itms.sh "$@")
	  else
	    {{engine}} run --rm -v "${scratch}:/work:z" -w /work "{{toolchain_image}}" sh ./itms.sh "$@"
	  fi
	}

	# A makeself archive that carries its own SHA-256 and MD5 manifests.
	echo "==> checking the archive"
	run_installer --check >/dev/null \
	  || { echo "error: the Transporter archive failed its integrity check" >&2; exit 1; }

	# --noexec skips Apple's install_script.sh, which only prompts, copies to
	# /usr/local/itms and chowns. Extracting is the same result without root.
	echo "==> extracting"
	run_installer --noexec --keep --nochown --target ./payload >/dev/null
	payload="${scratch}/payload"
	[ -x "${payload}/itms/bin/iTMSTransporter" ] \
	  || { echo "error: the archive contained no itms/bin/iTMSTransporter" >&2; exit 1; }

	# Section 3.6: the licence binds only when a person accepts it, so the
	# prompt Apple's installer would have shown is kept rather than routed around.
	if [ -z "${HALOGEN_TRANSPORTER_ACCEPT:-}" ]; then
	  "${PAGER:-more}" "${payload}/License.txt"
	  printf 'Do you agree to the above license terms? [yes or no] '
	  read -r reply
	  case "${reply}" in y*|Y*) ;; *) echo "not installed"; exit 1 ;; esac
	fi

	rm -rf "${dir}"
	mkdir -p "$(dirname "${dir}")"
	mv "${payload}/itms" "${dir}"
	cp "${payload}/License.txt" "${dir}/License.txt"
	echo "==> installed to ${dir}"
	# Proves the bundled JRE runs where the upload will actually use it.
	transporter_version() {
	  if [ "${HALOGEN_SIGNING_RUNTIME:-container}" = native ]; then
	    "$dir/bin/iTMSTransporter" -version
	  else
	    {{engine}} run --rm -v "${dir}:/opt/itms:ro,z" "{{toolchain_image}}" /opt/itms/bin/iTMSTransporter -version
	  fi
	}
	transporter_version 2>&1 | grep -F "iTMSTransporter, version" \
	  || { echo "error: the extracted Transporter does not run" >&2; exit 1; }

# Needs ASC_API_KEY_ID, ASC_API_ISSUER_ID and the matching .p8 (.env), plus a
# Forgejo token and `just ios-transporter-setup`. TAG defaults to ios-v<workspace
# version>. HALOGEN_UPLOAD_YES=1 skips the confirmation.
# Send a release's signed IPA to TestFlight from the toolchain container.
ios-upload-testflight-local tag="":
	#!/usr/bin/env bash
	set -euo pipefail
	tag='{{tag}}'
	# Default to the workspace version, the same extraction ios-sign-local uses,
	# so uploading follows the tree without repeating the version.
	if [ -z "${tag}" ]; then
	  version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml \
	    | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -n1)"
	  [ -n "${version}" ] \
	    || { echo "error: workspace version not found in Cargo.toml" >&2; exit 1; }
	  tag="ios-v${version}"
	  echo "no tag given; using ${tag} from the workspace version"
	fi
	[[ "${tag}" =~ ^ios-v[0-9]+\.[0-9]+\.[0-9]+$ ]] \
	  || { echo "error: tag must look like ios-v1.2.3" >&2; exit 2; }
	version="${tag#ios-v}"
	case "${HALOGEN_SIGNING_RUNTIME:-container}" in
	  native) [ "$(uname -s)" = Linux ] || { echo "error: native signing requires Linux" >&2; exit 2; } ;;
	  container) [ -n "{{toolchain_image}}" ] || { echo "error: set TOOLCHAIN_IMAGE or SERVICES_ROOT_DOMAIN" >&2; exit 1; } ;;
	  *) echo "error: HALOGEN_SIGNING_RUNTIME must be container or native" >&2; exit 2 ;;
	esac
	token="$(just _fj-token)"
	for var in ASC_API_KEY_ID ASC_API_ISSUER_ID; do
	  [ -n "${!var:-}" ] || { echo "error: ${var} is required (see .env.example)" >&2; exit 1; }
	done
	key="${ASC_API_KEY_PATH:-${HOME}/.appstoreconnect/private_keys/AuthKey_${ASC_API_KEY_ID}.p8}"
	[ -r "${key}" ] || { echo "error: cannot read the App Store Connect key: ${key}" >&2; exit 1; }
	[[ "$(stat -c '%a' "${key}")" =~ ^[0-7]?00$ ]] \
	  || { echo "error: ${key} must not be group/world accessible" >&2; exit 1; }
	[ -x '{{transporter_dir}}/bin/iTMSTransporter' ] \
	  || { echo "error: run 'just ios-transporter-setup' first" >&2; exit 1; }

	# Kept apart from the signing lane's directory: that one holds the build this
	# was made from, and wiping it here would take the signer's evidence with it.
	work='{{signing_dir}}'"/${tag}-upload"
	rm -rf "${work}"
	mkdir -p "${work}"
	chmod 700 "${work}"

	run_uploader() {
	  HALOGEN_SIGNING_IMAGE="{{toolchain_image}}" HALOGEN_SIGNING_ENGINE="{{engine}}" \
	    HALOGEN_TRANSPORTER_DIR="{{transporter_dir}}" ci/internal/signing/run-local.sh upload "$work" "$@"
	}
	run_uploader --check-tools

	echo "==> resolving ${tag}"
	release_id="$(just _fj-api GET "releases/tags/${tag}" | jq -er '.id')"
	assets="$(just _fj-api GET "releases/${release_id}/assets")"
	fetch() {
	  url="$(printf '%s' "${assets}" \
	    | jq -er --arg n "$1" '.[] | select(.name == $n) | .browser_download_url')" \
	    || { echo "error: $1 is not attached to ${tag}; sign and verify it first" >&2; exit 1; }
	  curl -fsSL --max-time 900 -H "Authorization: token ${token}" -o "${work}/$1" "${url}"
	}
	ipa="halogen-${version}-ios.ipa"
	# Transporter cannot analyse an app on Linux; ios-verify-signing produces
	# this description from the same IPA on macOS.
	plist="halogen-${version}-ios-appstoreinfo.plist"
	echo "==> downloading the signed build"
	fetch "${ipa}"
	fetch "${ipa}.sha256"
	fetch "${plist}"

	# The key is read here and handed over as environment, so neither its host
	# path nor its contents appear in the container's argv.
	ASC_API_KEY_BASE64="$(base64 -w0 < "${key}")"
	export ASC_API_KEY_BASE64
	# A build App Store Connect accepts can be expired but never deleted, and it
	# consumes its build number for good, so the last gate is a person. Apple
	# offers no package dry run for an app, so there is nothing to show first.
	echo
	echo "About to deliver ${ipa} to App Store Connect as key ${ASC_API_KEY_ID}."
	echo "sha256: $(awk '{print $1}' "${work}/${ipa}.sha256")"
	echo "ios-verify-signing should have passed for ${tag} before this."
	if [ -z "${HALOGEN_UPLOAD_YES:-}" ]; then
	  printf 'Upload to TestFlight? [y/N] '
	  read -r answer
	  case "${answer}" in y|Y|yes|YES) ;; *) echo "upload cancelled"; exit 1 ;; esac
	fi

	# Validation runs before Transporter in both execution modes.
	run_uploader "${ipa}" "${version}" "${plist}"

# ── Dev server (dev-*) ────────────────────────────────────────────────────────

# Server in dev config: devdata db + logs + maps, dev secret, seed data.
dev-server:
	mkdir -p "{{devdata}}/media" "{{devdata}}/logs"
	cargo run -p halogen-server -- \
	  --db-path "{{devdata}}/halogen.db" \
	  --media-root "{{devdata}}/media" \
	  --log-file "{{devdata}}/logs/halogen.log" \
	  --enable-public-server --public-root "{{dist}}" \
	  --admin-username dev --admin-password dev \
	  --auth-token-secret dev-secret \
	  --dev-use-mock-download \
	  --dev-seed-data

# dev-server, rebuilt + restarted on any Rust change (needs watchexec).
dev-watch:
	watchexec -w crates -e rs -r -- just dev-server

# Wipe the dev database (and its WAL sidecars); next boot re-migrates + re-seeds.
dev-db-reset:
	rm -f "{{devdata}}/halogen.db" "{{devdata}}/halogen.db-wal" "{{devdata}}/halogen.db-shm"
	@echo "dev db wiped"

# ── Build (build-*) ───────────────────────────────────────────────────────────

# Optimized server binary (embeds the current dist/) → target/release.
build-release: ui-build
	cargo build -p halogen-server --release --features embed-frontend

# ── Docker images (docker-build-*) ────────────────────────────────────────────

# Both images: toolchain then dev.
docker-build: docker-build-toolchain docker-build-dev

# Toolchain image → halogen-toolchain:local (base is in-cluster; set CI_BASE_IMAGE).
docker-build-toolchain:
	#!/usr/bin/env bash
	set -euo pipefail
	base="{{ci_base_image}}"
	[ -n "$base" ] || { echo "error: set CI_BASE_IMAGE or SERVICES_ROOT_DOMAIN (.env)" >&2; exit 1; }
	{{engine}} build -f ci/docker/toolchain/Dockerfile \
	  --build-arg BASE_IMAGE="$base" \
	  -t halogen-toolchain:local .

# Dev runtime image → halogen:dev (host-compiled binary staged as the context).
docker-build-dev: build-release
	#!/usr/bin/env bash
	set -euo pipefail
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/')"
	stage="{{cargo_target}}/docker-dev-context"
	mkdir -p "$stage"
	cp "{{cargo_target}}/release/halogen-server" "$stage/halogen-server"
	{{engine}} build -f ci/docker/dev/Dockerfile \
	  ${DEV_BASE_IMAGE:+--build-arg BASE_IMAGE="$DEV_BASE_IMAGE"} \
	  --build-arg VERSION="$version" \
	  -t halogen:dev "$stage"

# ── Dockerized commands (docker-*) ────────────────────────────────────────────
# Native recipes run inside the toolchain image; target cache in a named volume.
# dev-* recipes stay host-only except docker-dev-server below.

# Run any recipe in the toolchain container, e.g. `just docker-run test-one restart`.
docker-run +cmd:
	#!/usr/bin/env bash
	set -euo pipefail
	[ -n "{{toolchain_image}}" ] || { echo "error: set TOOLCHAIN_IMAGE or SERVICES_ROOT_DOMAIN (.env)" >&2; exit 1; }
	# --init: chrome's tree reparents to pid 1 when chromedriver dies, so
	# without a reaper the e2e teardown leaves zombies behind.
	{{engine}} run --rm --init \
	  -v "{{justfile_directory()}}:/src:z" -w /src \
	  -v halogen-docker-target:/docker-target \
	  -e CARGO_TARGET_DIR=/docker-target \
	  -e CARGO_REGISTRIES_CHILLED_PROXY_INDEX \
	  "{{toolchain_image}}" \
	  just {{cmd}}

# CI-parity mode keeps its own target+sccache volumes and disables incremental
# compilation. Cargo home stays image-owned so its proxy config cannot be hidden.
# Run any recipe with the stable CI-parity cache/profile settings.
docker-run-ci +cmd:
	#!/usr/bin/env bash
	set -euo pipefail
	[ -n "{{toolchain_image}}" ] || { echo "error: set TOOLCHAIN_IMAGE or SERVICES_ROOT_DOMAIN (.env)" >&2; exit 1; }
	{{engine}} run --rm --init \
	  -v "{{justfile_directory()}}:/src:z" -w /src \
	  -v halogen-docker-ci-target:/docker-target \
	  -v halogen-docker-sccache:/docker-sccache \
	  -e CARGO_TARGET_DIR=/docker-target \
	  -e CARGO_INCREMENTAL=0 \
	  -e RUSTC_WRAPPER=sccache \
	  -e "CC=sccache cc" -e "CXX=sccache c++" \
	  -e SCCACHE_DIR=/docker-sccache -e SCCACHE_CACHE_SIZE=40G \
	  -e CARGO_PROFILE_DEV_DEBUG=0 -e CARGO_PROFILE_TEST_DEBUG=0 \
	  -e CARGO_REGISTRIES_CHILLED_PROXY_INDEX \
	  "{{toolchain_image}}" \
	  just {{cmd}}

# Server + UI in dev mode inside the toolchain container (UI built first;
# port published; override with HALOGEN_LISTEN_PORT).
docker-dev-server:
	#!/usr/bin/env bash
	set -euo pipefail
	[ -n "{{toolchain_image}}" ] || { echo "error: set TOOLCHAIN_IMAGE or SERVICES_ROOT_DOMAIN (.env)" >&2; exit 1; }
	port="${HALOGEN_LISTEN_PORT:-8080}"
	{{engine}} run --rm -it \
	  -p "${port}:${port}" \
	  -v "{{justfile_directory()}}:/src:z" -w /src \
	  -v halogen-docker-target:/docker-target \
	  -e CARGO_TARGET_DIR=/docker-target \
	  -e CARGO_REGISTRIES_CHILLED_PROXY_INDEX \
	  -e HALOGEN_LISTEN_ADDRESS=0.0.0.0 \
	  -e HALOGEN_LISTEN_PORT="${port}" \
	  "{{toolchain_image}}" \
	  just ui-build dev-server

docker-check: (docker-run "check")
docker-check-wasm: (docker-run "check-wasm")
docker-check-all: (docker-run "check-all")
docker-clippy: (docker-run "clippy")
docker-fmt: (docker-run "fmt")
docker-fmt-check: (docker-run "fmt-check")
docker-test: (docker-run "test")
docker-test-crates: (docker-run "test-crates")
docker-test-webui: (docker-run "test-webui")
docker-test-one filter: (docker-run "test-one" filter)
docker-test-doc: (docker-run "test-doc")
docker-test-e2e: (docker-run "test-e2e")
docker-build-release: (docker-run "build-release")

# Explicit containerized CI-parity variants; interactive docker-* stays incremental.
docker-ci-check: (docker-run-ci "check")
docker-ci-check-wasm: (docker-run-ci "check-wasm")
docker-ci-check-all: (docker-run-ci "check-all")
docker-ci-clippy: (docker-run-ci "clippy")
docker-ci-test: (docker-run-ci "test")
docker-ci-test-crates: (docker-run-ci "test-crates")
docker-ci-test-webui: (docker-run-ci "test-webui")
docker-ci-test-one filter: (docker-run-ci "test-one" filter)
docker-ci-test-doc: (docker-run-ci "test-doc")
docker-ci-test-e2e: (docker-run-ci "test-e2e")
docker-ci-build-release: (docker-run-ci "build-release")

# ── Version bump (version-bump-*) ────────────────────────────────────────────
# Bump [workspace.package], then refresh Cargo.lock before a tagged release.

version-bump-bugfix: (_version-bump "patch")
version-bump-minor: (_version-bump "minor")
version-bump-major: (_version-bump "major")

# ── CI triggers (ci-* — push a tag → a workflow run) ──────────────────────────
# Every recipe pushes to `internal` (the Forgejo remote); CI builds the tagged
# commit, so anything uncommitted is absent from the resulting image.

# iOS ships separately via `just ci-tagged-release-ios`; bump the workspace version + commit first.
# Push v<version> → release.yml: image :<version> + :latest AND the Forgejo release with assets.
ci-tagged-release:
	#!/usr/bin/env bash
	set -euo pipefail
	echo "Formatting..."
	cargo fmt --all
	# Same extraction release.yml validates the tag against — keep them in step.
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -n1)"
	[ -n "${version}" ] || { echo "error: workspace version not found in Cargo.toml" >&2; exit 1; }
	tag="v${version}"
	# Tag must point at a clean, pushed commit. Cargo.lock drift is exempt but does
	# NOT ship: CI builds from the COMMITTED lock at the tag.
	if [ -n "$(git status --porcelain -- ':!Cargo.lock')" ]; then
	  echo "error: working tree dirty (besides Cargo.lock); commit or stash before tagging ${tag}" >&2
	  exit 1
	fi
	if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
	  echo "error: tag ${tag} already exists (bump the workspace version first)" >&2
	  exit 1
	fi
	git tag -a "${tag}" -m "Release ${tag}"
	git push internal "${tag}"
	echo "Pushed ${tag} -> CI builds & pushes halogen:${version} (+ :latest) and publishes the Forgejo release (linux server binary + sha256)"

# Push ios-v<workspace-version> to build and publish the iOS release. Its own
# tag prefix keeps it independent of the server release lane.
ci-tagged-release-ios:
	#!/usr/bin/env bash
	set -euo pipefail
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -n1)"
	[[ "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
	  echo "error: iOS releases require an App Store version like 1.2.3, found '${version:-missing}'" >&2
	  exit 1
	}
	tag="ios-v${version}"
	if [ -n "$(git status --porcelain -- ':!Cargo.lock')" ]; then
	  echo "error: working tree dirty (besides Cargo.lock); commit or stash before tagging ${tag}" >&2
	  exit 1
	fi
	if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
	  echo "error: tag ${tag} already exists (bump the workspace version first)" >&2
	  exit 1
	fi
	git tag -a "${tag}" -m "iOS release ${tag}"
	git push internal "${tag}"
	echo "Pushed ${tag} -> ios-release.yml runs the e2e gate, archives the unsigned app, and publishes the release"

# Re-release the CURRENT iOS version from HEAD: no clean-tree check, tag
# overwritten (force push re-fires ios-release.yml). CI builds the tagged
# commit, so uncommitted work still does NOT ship.
ci-tagged-release-ios-force:
	#!/usr/bin/env bash
	set -euo pipefail
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -n1)"
	[[ "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
	  echo "error: iOS releases require an App Store version like 1.2.3, found '${version:-missing}'" >&2
	  exit 1
	}
	tag="ios-v${version}"
	if [ -n "$(git status --porcelain -- ':!Cargo.lock')" ]; then
	  echo "warning: working tree dirty; CI builds the tagged commit, not local changes" >&2
	fi
	git tag -fa "${tag}" -m "iOS release ${tag}"
	git push --force internal "refs/tags/${tag}:refs/tags/${tag}"
	echo "Force-pushed ${tag} -> ios-release.yml re-runs the gate and re-publishes the release"

# Uncommitted work still does NOT ship — CI builds the tagged commit.
# Re-release the CURRENT version from HEAD: no clean-tree check, tag overwritten (force push re-fires release.yml).
ci-tagged-release-force:
	#!/usr/bin/env bash
	set -euo pipefail
	echo "Formatting..."
	cargo fmt --all
	# Same extraction release.yml validates the tag against — keep them in step.
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -n1)"
	[ -n "${version}" ] || { echo "error: workspace version not found in Cargo.toml" >&2; exit 1; }
	tag="v${version}"
	if [ -n "$(git status --porcelain)" ]; then
	  echo "warning: working tree dirty; CI builds the tagged commit, not your local changes" >&2
	fi
	git tag -fa "${tag}" -m "Release ${tag}"
	git push -f internal "${tag}"
	echo "Force-pushed ${tag} -> CI rebuilds halogen:${version} (+ :latest) and republishes the Forgejo release"

# Tag-triggered CI runs, one per tier. Unlike ci-tagged-release these do NOT
# require a clean tree — they are for exercising a tier where the runner has
# what this machine lacks (chromium for the browser tier, above all). CI builds
# the tagged commit, so commit first or the run tests something else.
_ci-tag recipe:
	#!/usr/bin/env bash
	set -euo pipefail
	tag="ci-{{recipe}}-$(date +%Y%m%d-%H%M%S)"
	if [ -n "$(git status --porcelain)" ]; then
	  echo "warning: working tree dirty; CI runs the tagged commit, not your local changes" >&2
	fi
	git tag "${tag}"
	git push internal "${tag}"
	echo "Pushed ${tag} -> {{recipe}}.yml"

# `just ci-test-e2e` is the one that matters most: the browser tier cannot run
# without chromium, which CI has and a dev box generally does not.
# Run one tier in CI (check, clippy, test-crates, test-webui, test-e2e).
ci-check: (_ci-tag "check")
ci-clippy: (_ci-tag "clippy")
ci-test-crates: (_ci-tag "test-crates")
ci-test-webui: (_ci-tag "test-webui")
ci-test-e2e: (_ci-tag "test-e2e")

# Push dev-<ts> → dev-release.yml (same tests, debug image :dev-<version> + :dev).
ci-dev-release:
	#!/usr/bin/env bash
	set -euo pipefail
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -n1)"
	[ -n "${version}" ] || { echo "error: workspace version not found in Cargo.toml" >&2; exit 1; }
	# Deliberately non-blocking — dev tags are cheap — but the build omits
	# uncommitted work, so say so loudly.
	if [ -n "$(git status --porcelain)" ]; then
	  echo "warning: working tree dirty; CI builds the tagged commit, not your local changes" >&2
	fi
	tag="dev-$(date +%Y%m%d-%H%M%S)"
	git tag "${tag}"
	git push internal "${tag}"
	echo "Pushed ${tag} -> CI builds & pushes halogen:dev-${version} (+ :dev)"

# Forgejo wrappers use ci/fj/fj.sh; its help lists subcommands.
# Resolve host/repo from FORGEJO_HOST/FORGEJO_REPO_PATH or the remote URL, and token from env, .env, or fj credentials.

# Remote whose URL supplies the host and owner/repo — nothing is hardcoded.
fj_remote := env("HALOGEN_FJ_REMOTE", "internal")

# GET a repo-relative API path. `just fj-api "issues?state=open"`.
fj-api path:
	@ci/fj/fj.sh api GET {{quote(path)}}

# host/owner/repo behind the remote — the ios release recipes address through
# this. ssh://, scp-style and https:// URLs all reduce to the same thing.
_fj-slug:
	#!/usr/bin/env bash
	set -euo pipefail
	git remote get-url '{{fj_remote}}' \
	  | sed -E 's#^[a-z+]+://##; s#^[^@/]*@##; s#:#/#; s#\.git$##'

# The application token for this host: the environment first, else whatever
# `fj` already stored when it logged in, so a working CLI needs no second setup.
_fj-token:
	#!/usr/bin/env bash
	set -euo pipefail
	if [ -n "${FORGEJO_TOKEN:-}" ]; then printf '%s\n' "${FORGEJO_TOKEN}"; exit 0; fi
	keys="${FORGEJO_CLI_KEYS:-${XDG_DATA_HOME:-${HOME}/.local/share}/forgejo-cli/keys.json}"
	host="$(just _fj-slug)"; host="${host%%/*}"
	token="$(jq -er --arg h "${host}" '.hosts[$h].token // empty' "${keys}" 2>/dev/null || true)"
	[ -n "${token}" ] || {
	  echo "error: set FORGEJO_TOKEN, or log in with fj (no token for ${host} in ${keys})" >&2
	  exit 1
	}
	printf '%s\n' "${token}"

_fj-api method path:
	@ci/fj/fj.sh api {{quote(method)}} {{quote(path)}}

# `just fj-dispatch ios-verify-signing.yml master '{"tag":"ios-v0.3.1"}'`
# Fire a workflow_dispatch run. INPUTS is a JSON object of string values.
fj-dispatch workflow ref="master" inputs="{}":
	@ci/fj/fj.sh dispatch {{quote(workflow)}} {{quote(ref)}} {{quote(inputs)}}

# Run rows for one workflow file, newest first: run/status/job/event/sha.
# WORKFLOW is the file name, e.g. test-e2e.yml — NOT the `name:` inside it.
fj-runs workflow job="":
	@ci/fj/fj.sh runs {{quote(workflow)}} {{quote(job)}}

# Highest run number for a workflow — the watermark to take BEFORE dispatching.
fj-run-latest workflow:
	@ci/fj/fj.sh run-latest {{quote(workflow)}}

# Block until WORKFLOW's JOB reaches a terminal state in a run above SINCE.
# Matching on workflow AND job is the point: `apple-build` alone also matches
# flanforge.yml's nightly, which would hand back someone else's verdict.
fj-run-wait workflow job since poll="30" tries="240":
	@ci/fj/fj.sh run-wait {{quote(workflow)}} {{quote(job)}} {{quote(since)}} {{quote(poll)}} {{quote(tries)}}

# Open issues this CI opened. Filtered locally, not by the server: Forgejo
# ignores a `labels=` filter naming a label that does not exist.
fj-ci-issues state="open":
	@ci/fj/fj.sh issues {{quote(state)}}

# The issue a run opened, by run number — exact, never fuzzy title search.
fj-issue-for-run run:
	@ci/fj/fj.sh issue-for-run {{quote(run)}}

# Body and append-only comments of one issue — the complete run evidence.
fj-issue number:
	@ci/fj/fj.sh issue {{quote(number)}}

# Comment why, then close — the issue is the durable record, not the run.
fj-issue-close number comment:
	@ci/fj/fj.sh issue-close {{quote(number)}} {{quote(comment)}}

# Download an issue's attachments into DEST (default /tmp/ci-<number>).
fj-issue-artifacts number dest="":
	@ci/fj/fj.sh issue-artifacts {{quote(number)}} {{quote(dest)}}

# ── Halogen platform and podcast extensions ─────────────────────────────────

android_dir := justfile_directory() / "android"
_dd_curl := "curl -fsS -H \"X-Api-Key: $DROIDDRIVER_KEY\""

_npm-install:
	#!/usr/bin/env bash
	set -euo pipefail
	cd "{{uidir}}"
	stamp="$(sha256sum package.json package-lock.json | sha256sum | cut -d' ' -f1)"
	if [ ! -d node_modules/daisyui ] || [ "$(cat node_modules/.halogen-inputs 2>/dev/null || true)" != "$stamp" ]; then
	  npm ci
	  printf '%s\n' "$stamp" > node_modules/.halogen-inputs
	fi

_ios-bindings: ios-wire-types
	#!/usr/bin/env bash
	set -euo pipefail
	# Keep the host library on the bindgen feature set. Toggling it off and on
	# forces two uncacheable mobile-ffi rebuilds on every invocation.
	node "{{ios_build_timing}}" cargo-host -- \
	  cargo build --timings -p halogen-mobile-ffi --features bindgen --lib --bin uniffi-bindgen
	# The host cdylib is .dylib on macOS, .so on the Linux dev loop.
	host_lib="{{cargo_target}}/debug/libhalogen_mobile.dylib"
	[ -e "$host_lib" ] || host_lib="${host_lib%.dylib}.so"
	node "{{ios_build_timing}}" uniffi -- ci/internal/mac/ios-uniffi-bindgen.sh \
	  "{{cargo_target}}/debug/uniffi-bindgen" \
	  "$host_lib" \
	  "{{ios_dir}}/Generated" "{{ios_dir}}/.cache/uniffi-bindgen-debug.stamp"
	node "{{ios_build_timing}}" header-install -- \
	  '{{install_if_changed}}' "{{ios_dir}}/Generated/halogen_mobileFFI.h" "{{ios_dir}}/Rust/include/halogen_mobileFFI.h"

ios-device-server: ios-e2e-server
	ci/internal/device/run-device-server.sh "{{cargo_target}}/debug/halogen-server"

ios-appicon:
	python3 data/assets/generate-icons.py --ios-only

design-build:
	python3 design/build-master.py

design-render *PAGES:
	design/render.sh {{PAGES}}

test-integ:
	cargo nextest run -p halogen-integ --no-fail-fast

lighthouse *ARGS: ui-build
	cargo run -p halogen-tool-lighthouse {{ARGS}}

lighthouse-ext url *ARGS:
	cargo run -p halogen-tool-lighthouse -- --base-url {{url}} {{ARGS}}

ui-desktop-build *ARGS: tailwind
	cd "{{uidir}}" && RUSTC_WRAPPER="" dx build --package halogen-webui --platform desktop --no-default-features --features desktop {{ARGS}}

ui-desktop-serve: tailwind
	cd "{{uidir}}" && RUSTC_WRAPPER="" dx serve --package halogen-webui --platform desktop --no-default-features --features desktop

ui-windows-build *ARGS: tailwind
	cd "{{uidir}}" && RUSTC_WRAPPER="" dx build --package halogen-webui --platform desktop --target x86_64-pc-windows-gnu --profile windows-release --no-default-features --features desktop {{ARGS}}

android-apk-collect: (android-build "release")
	rm -rf "{{justfile_directory()}}/artifacts/release/android"
	mkdir -p "{{justfile_directory()}}/artifacts/release/android"
	cp "{{android_dir}}/app/build/outputs/apk/release/app-release.apk" "{{justfile_directory()}}/artifacts/release/android/halogen.apk"

android-build PROFILE="debug": (android-core PROFILE)
	#!/usr/bin/env bash
	set -euo pipefail
	version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "{{justfile_directory()}}/Cargo.toml" | head -n1)"
	# Minutes since epoch: monotonic like ios-archive's minute stamp, but
	# fits Android's Int32 versionCode (yymmddHHMM overflows it).
	code="$(( $(date -u +%s) / 60 ))"
	cd "{{android_dir}}"
	./gradlew --no-daemon ":app:assemble{{ if PROFILE == "release" { "Release" } else { "Debug" } }}" -PversionName="$version" -PversionCode="$code"

android-core PROFILE="debug": android-wire-types
	# Generate bindings from an unstripped host library; Android keeps its requested profile.
	cargo build -p halogen-mobile-ffi --features bindgen --lib --bin uniffi-bindgen
	cargo ndk --platform 30 -t x86_64 -t arm64-v8a -o "{{android_dir}}/app/src/main/jniLibs" build -p halogen-mobile-ffi {{ if PROFILE == "release" { "--release" } else { "" } }}
	"{{cargo_target}}/debug/uniffi-bindgen" generate --library "{{cargo_target}}/debug/libhalogen_mobile.so" --language kotlin --out-dir "{{android_dir}}/generated/uniffi" --no-format

android-e2e CLASS="org.fgsec.halogen.RemoteJourneyTest" PORT="8099":
	#!/usr/bin/env bash
	set -euo pipefail
	out="{{justfile_directory()}}/data/run/android/e2e"
	rm -rf "$out" && mkdir -p "$out"
	e2etmp="$(mktemp -d)"
	mkdir -p "$e2etmp/media" "$e2etmp/public/feeds"
	python3 ci/internal/lib/seed-discover-fixtures.py "$e2etmp/public" '{{PORT}}'
	export HALOGEN_DISCOVER_ITUNES_BASE_URL="http://127.0.0.1:{{PORT}}/discover/itunes.json"
	export HALOGEN_DISCOVER_GPODDER_BASE_URL="http://127.0.0.1:{{PORT}}/discover/gpodder.json"
	export EMU_SERVER="$DROIDDRIVER_URL"
	export EMU_API_KEY="$DROIDDRIVER_KEY"
	cargo build -p halogen-server
	# A hard-killed previous run skips its trap and leaves its server holding
	# the port; the new one then silently loses the bind race and the whole
	# run executes against STALE data. Clear the port first (/proc scan —
	# pkill/pgrep are not guaranteed in the sandbox).
	for c in /proc/[0-9]*/cmdline; do
	  if tr '\0' ' ' < "$c" 2>/dev/null | grep -q "halogen-server .*--listen-port {{PORT}}"; then
	    p="${c#/proc/}"; kill "${p%/cmdline}" 2>/dev/null || true
	  fi
	done
	sleep 1
	# Seed from the sanitized temporary data/tests tree.
	server_binary="$(python3 -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).resolve())' '{{cargo_target}}/debug/halogen-server')"
	(cd "$e2etmp"; exec "$server_binary" \
	  --db-path "$e2etmp/halogen.db" \
	  --media-root "$e2etmp/media" \
	  --log-file "$out/server.log" \
	  --enable-public-server --public-root "$e2etmp/public" \
	  --admin-username dev --admin-password dev \
	  --auth-token-secret dev-secret \
	  --discover-itunes-base-url "$HALOGEN_DISCOVER_ITUNES_BASE_URL" \
	  --discover-gpodder-base-url "$HALOGEN_DISCOVER_GPODDER_BASE_URL" \
	  --listen-port {{PORT}} --allow-private-network \
	  --dev-use-mock-download \
	  --dev-seed-data) &
	server_pid=$!
	tunnel_pid=""
	cleanup() {
	  for pid in "$tunnel_pid" "$server_pid"; do
	    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
	  done
	  rm -rf "$e2etmp"
	}
	trap cleanup EXIT
	for _ in $(seq 1 120); do
	  curl -sf -m 1 "http://127.0.0.1:{{PORT}}/healthz" >/dev/null && break
	  sleep 0.5
	done
	curl -sf -m 2 "http://127.0.0.1:{{PORT}}/healthz" >/dev/null || { echo "error: server never came up" >&2; exit 1; }
	# healthz alone can't tell OUR server from a stale one answering the port.
	kill -0 "$server_pid" 2>/dev/null || { echo "error: server exited (port already taken?)" >&2; exit 1; }
	# The device is stopped after an idle auto-stop or a pod roll, and /readyz
	# does not wake it — `wait` alone would just burn its timeout. `start` on a
	# running device is a no-op that reports the current state.
	emucli start
	emucli wait --timeout 600
	emucli tunnel --guest-port {{PORT}} --local "127.0.0.1:{{PORT}}" &
	tunnel_pid=$!
	# A ~165 MB upload occasionally dies mid-write with a 502. emucli says which
	# failures are worth retrying (502 yes, 409 no) — honour exactly that, so a
	# genuine refusal still fails on the first try.
	install_apk() {
	  # `local`: a bare assignment here would clobber $out, the artifacts dir.
	  local reply
	  for _ in 1 2 3; do
	    if reply="$(emucli install "$1" 2>&1)"; then printf '%s\n' "$reply"; return 0; fi
	    printf '%s\n' "$reply" >&2
	    case "$reply" in *"worth a retry"*) sleep 5 ;; *) return 1 ;; esac
	  done
	  return 1
	}
	# Full pm clear: HALOGEN_RESET wipes selectively, and state left by a
	# PREVIOUS run (same account namespace against a re-seeded server) bled
	# into journeys as stale caches and stale outbox ops.
	emucli reset org.fgsec.halogen 2>/dev/null || true
	install_apk "{{android_dir}}/app/build/outputs/apk/debug/app-debug.apk"
	install_apk "{{android_dir}}/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"
	set +e
	emucli test org.fgsec.halogen.test --class '{{CLASS}}' \
	  -e "HALOGEN_E2E_BASE=http://10.0.2.2:{{PORT}}" --timeout 1500 | tee "$out/run.log"
	status=${PIPESTATUS[0]}
	set -e
	# Journey screenshots (every step, pass or fail) — named after the step.
	journey=/sdcard/Android/data/org.fgsec.halogen/files/journey
	# `emucli ls` prints "<kind> <size>  <path>"; step names contain spaces, so
	# strip exactly that prefix rather than splitting on whitespace.
	emucli ls "$journey" 2>/dev/null | sed -E 's/^[d-] +[0-9]+  //' |
	  while read -r f; do emucli pull "$f" -o "$out/$(basename "$f")" >/dev/null || true; done
	echo "artifacts: $out"
	exit $status

android-screenshots:
	#!/usr/bin/env bash
	set -euo pipefail
	out="{{justfile_directory()}}/design/screenshots/android/latest"
	rm -rf "$out" && mkdir -p "$out/remote" "$out/embedded"
	just android-e2e
	cp "{{justfile_directory()}}/data/run/android/e2e/"*.png "$out/remote/"
	just android-e2e org.fgsec.halogen.EmbeddedJourneyTest
	cp "{{justfile_directory()}}/data/run/android/e2e/"*.png "$out/embedded/"
	# Journey step snaps only — failure shots never belong in review artifacts.
	rm -f "$out"/*/*MISSING*.png
	echo "screenshots: $out"

android-test-build:
	cd "{{android_dir}}" && ./gradlew --no-daemon :app:assembleDebugAndroidTest

android-wire-types:
	mkdir -p "{{android_dir}}/generated/wire"
	typeshare --lang=kotlin --config-file typeshare.toml --output-file "{{android_dir}}/generated/wire/WireTypes.kt" crates/wire crates/wire-meta

dd-install APK="" LAUNCH="org.fgsec.halogen":
	#!/usr/bin/env bash
	set -euo pipefail
	apk='{{APK}}'
	[ -n "$apk" ] || apk="{{android_dir}}/app/build/outputs/apk/debug/app-debug.apk"
	{{_dd_curl}} --data-binary @"$apk" "$DROIDDRIVER_URL/apk?launch={{LAUNCH}}"

dd-ready:
	{{_dd_curl}} --retry 60 --retry-delay 5 --retry-all-errors "$DROIDDRIVER_URL/readyz"

dd-screenshot FILE="artifacts/dd/screen.png":
	mkdir -p "$(dirname "{{FILE}}")"
	{{_dd_curl}} "$DROIDDRIVER_URL/screenshot.png" -o "{{FILE}}"
	@echo "wrote {{FILE}}"

dd-test PACKAGE="org.fgsec.halogen.test" CLASS="" TIMEOUT="1800":
	#!/usr/bin/env bash
	set -euo pipefail
	url="$DROIDDRIVER_URL/api/test?package={{PACKAGE}}&timeout={{TIMEOUT}}"
	[ -z "{{CLASS}}" ] || url="$url&class={{CLASS}}"
	{{_dd_curl}} -N -X POST "$url"

# The podcast sync worker has its own WASM artifact and classic-worker loader.
_worker-build OUT: _ui-check-wbg
	cargo build -p halogen-webui-worker --profile wasm-release --target wasm32-unknown-unknown
	wasm-bindgen --target no-modules --no-typescript --out-name halogen_worker \
	  --out-dir "{{OUT}}/worker" \
	  "{{cargo_target}}/wasm32-unknown-unknown/wasm-release/halogen_webui_worker.wasm"

check-desktop: tailwind
	cargo check -p halogen-webui --no-default-features --features desktop

# Standalone browser diagnostic walkthrough, distinct from CI journey captures.
web-screenshots-local *ARGS: ui-build
	cargo run -p halogen-tool-screenshot {{ARGS}}
