# Halogen task runner. Recipes grouped by prefix; `_` helpers up top.
# `just` (no args) lists the public recipes.

# Load the repo-root .env (if present) into every recipe; shell env wins over it.
set dotenv-load := true

# ── Variables ─────────────────────────────────────────────────────────────────

# Built web frontend (flat; git-ignored contents).
dist := justfile_directory() / "dist"
# Dev data: db + media + logs (git-ignored).
devdata := justfile_directory() / "data" / "devdata"
# UI crate dir — dx runs from here (Dioxus.toml lives here).
uidir := justfile_directory() / "crates" / "ui"
# Web-only features (keeps desktop deps out of wasm).
web_features := "--no-default-features --features web"
# Where dx emits the release web app (honors CARGO_TARGET_DIR).
web_out := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() / "target") / "dx" / "halogen-ui" / "release" / "web" / "public"
# Same, for a debug build.
web_out_debug := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() / "target") / "dx" / "halogen-ui" / "debug" / "web" / "public"
# Staging dir for `dx bundle --out-dir` (it appends `public/`).
bundle_stage := justfile_directory() / "target" / "dx-bundle"
# Cargo target dir (honors CARGO_TARGET_DIR).
cargo_target := env_var_or_default("CARGO_TARGET_DIR", justfile_directory() / "target")
# Sync Web Worker wasm — our own wasm-bindgen build (dx has no web-worker target, #3275).
worker_wasm := cargo_target / "wasm32-unknown-unknown" / "release" / "halogen_ui_worker.wasm"

# Service endpoints: explicit knob (.env.example) wins, else derived from
# SERVICES_ROOT_DOMAIN — no hostname lives in the repo.
services_root := env("SERVICES_ROOT_DOMAIN", "")
# deps-proxy crates index. The registry NAME must match the `replace-with` the
# build image bakes into $CARGO_HOME/config.toml (chilled-proxy) or cargo errors.
export CARGO_REGISTRIES_CHILLED_PROXY_INDEX := env("CARGO_REGISTRIES_CHILLED_PROXY_INDEX", if services_root == "" { "" } else { "sparse+https://deps." + services_root + "/crates/index/" })
# npm mount on the same proxy — a gate, so it fails closed too: unset, the
# sentinel host never resolves. Public users set NPM_REGISTRY (see .env.example).
export npm_config_registry := env("npm_config_registry", env("NPM_REGISTRY", if services_root == "" { "https://npm-registry-not-configured.invalid/" } else { "https://deps." + services_root + "/npm/" }))
# Toolchain image ref for docker builds (empty → the image recipes fail with a clear message).
toolchain_image := env("TOOLCHAIN_IMAGE", if services_root == "" { "" } else { "registry." + services_root + "/halogen-toolchain:latest" })

# ── Default ───────────────────────────────────────────────────────────────────

# Default: `just` with no args lists recipes.
default:
	@just --list

# ── Helpers (`_` — deps of other recipes; hidden from `just --list`) ───────────

# Flatten a dx web output + pwa/ into dist/ (PWA files at root for sw.js scope).
_sync-dist src:
	mkdir -p "{{dist}}"
	find "{{dist}}" -mindepth 1 ! -name .gitkeep -delete
	cp -r "{{src}}/." "{{dist}}/"
	cp -r "{{uidir}}/pwa/." "{{dist}}/"
	@# tailwind.css is git-ignored (dx skips it under .git); copy it ourselves.
	mkdir -p "{{dist}}/assets"
	cp "{{uidir}}/assets/tailwind.css" "{{dist}}/assets/tailwind.css"
	@# Pin SW cache key to the wasm fingerprint so each bundle evicts the last.
	@wasm="$(find "{{dist}}" -name '*_bg*.wasm' -not -path '*/worker/*' -printf '%f\n' 2>/dev/null | head -n1)"; if [ -n "$wasm" ]; then sed -i "s/\"halogen-v2\"/\"halogen-$wasm\"/" "{{dist}}/sw.js"; fi
	@# Fail loudly on an incomplete dist/ (else embed-frontend bakes an empty bundle).
	@test -f "{{dist}}/index.html" || { echo "ERROR: {{dist}}/index.html missing after sync from {{src}} — the dx build produced no index.html"; exit 1; }
	@find "{{dist}}" -name '*_bg*.wasm' -not -path '*/worker/*' | grep -q . || { echo "ERROR: no app *_bg.wasm anywhere in {{dist}} after sync from {{src}} — the dx wasm build is missing/incomplete (wasm-opt crash?)"; exit 1; }
	@test -f "{{dist}}/worker.js" || { echo "ERROR: {{dist}}/worker.js missing after sync — the sync Web Worker bootstrap wasn't copied (pwa/worker.js)"; exit 1; }
	@test -f "{{dist}}/worker/halogen_worker_bg.wasm" || { echo "ERROR: {{dist}}/worker/halogen_worker_bg.wasm missing after sync — run _worker-build before _sync-dist (the Web Worker wasm is missing)"; exit 1; }
	@echo "Synced {{src}} (+ pwa/ + assets/tailwind.css + worker/) -> {{dist}}"

# Build the sync Web Worker wasm + glue into "{{OUT}}/worker" (dx has no web-worker target, #3275).
# The wasm-bindgen CLI must match the locked crate EXACTLY (a mismatch panics at load) — read from Cargo.lock.
_worker-build OUT:
	#!/usr/bin/env bash
	set -euo pipefail
	# Exact locked wasm-bindgen crate version (the CLI must match it byte-for-byte).
	ver="$(awk '/^name = "wasm-bindgen"$/{f=1; next} f && /^version = /{gsub(/"/,"",$3); print $3; exit}' "{{justfile_directory()}}/Cargo.lock")"
	if [ -z "${ver:-}" ]; then
	  echo "error: could not read locked wasm-bindgen version from Cargo.lock" >&2
	  exit 1
	fi
	# Compile the worker cdylib for wasm.
	cargo build -p halogen-ui-worker --target wasm32-unknown-unknown --release
	# Find a wasm-bindgen CLI == $ver: dx's vendored copy, then PATH, else pin-install.
	data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
	wb="$data_home/.dx/tools/wasm-bindgen-$ver/wasm-bindgen"
	if [ ! -x "$wb" ]; then
	  if command -v wasm-bindgen >/dev/null 2>&1 && [ "$(wasm-bindgen --version | awk '{print $2}')" = "$ver" ]; then
	    wb="$(command -v wasm-bindgen)"
	  else
	    echo "wasm-bindgen $ver not found (dx tools or PATH); installing the pinned CLI..."
	    cargo install wasm-bindgen-cli --version "$ver" --locked
	    wb="wasm-bindgen"
	  fi
	fi
	echo "Using wasm-bindgen: $wb (v$ver)"
	"$wb" --target no-modules --no-typescript \
	  --out-name halogen_worker --out-dir "{{OUT}}/worker" \
	  "{{worker_wasm}}"
	echo "Built sync Web Worker -> {{OUT}}/worker"

# Ensure crates/ui/assets/tailwind.css exists for compile checks — real compile if the
# toolchain is present, else a placeholder (shipping recipes overwrite it via `tailwind`).
_tailwind-ensure:
	#!/usr/bin/env bash
	set -euo pipefail
	css="{{uidir}}/assets/tailwind.css"
	[ -f "$css" ] && exit 0
	if command -v tailwindcss >/dev/null 2>&1 && [ -d "{{uidir}}/node_modules" ]; then
	  just tailwind
	else
	  mkdir -p "{{uidir}}/assets"
	  printf '/* placeholder for compile checks — run `just tailwind` for real styles */\n' > "$css"
	  echo "note: created placeholder assets/tailwind.css (tailwindcss CLI or crates/ui/node_modules missing)"
	fi

# Shared impl behind version-bump-*. part ∈ {major,minor,patch}.
_version-bump part:
	#!/usr/bin/env bash
	set -euo pipefail
	part='{{part}}'
	# [workspace.package] version only (deps also have version= lines).
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
	# Rewrite only the first version line inside [workspace.package].
	awk -v new="$new" '
	  /^\[/ { inpkg = ($0 == "[workspace.package]") }
	  inpkg && /^version[[:space:]]*=/ && !done { sub(/"[^"]*"/, "\"" new "\""); done = 1 }
	  { print }
	' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
	echo "[workspace.package] version: ${cur} -> ${new}"
	echo "Next: cargo build (refresh Cargo.lock), commit, then 'just ci-tagged-release'."

# ── Frontend (tailwind + ui-*) ────────────────────────────────────────────────

# Tailwind + daisyUI → crates/ui/assets/tailwind.css (dx doesn't run Tailwind; needs the CLI).
tailwind:
	cd "{{uidir}}" && tailwindcss -i tailwind.css -o assets/tailwind.css --minify

# `--debug-symbols false` is REQUIRED: DWARF makes wasm-opt SIGABRT and dx ships a broken wasm.
# Release frontend → dist/ (wasm-opt'd) — used by dev-server + e2e.
ui-build: tailwind
	@# Clear stale content-hashed output (dx never prunes old hashes).
	rm -rf "{{web_out}}"
	cd "{{uidir}}" && dx build --platform web --profile wasm-release {{web_features}} --debug-symbols false
	@just _worker-build "{{web_out}}"
	@just _sync-dist "{{web_out}}"

# Debug frontend → dist/ — dev IMAGE only (no wasm-opt: faster, bigger wasm).
ui-build-debug: tailwind
	cd "{{uidir}}" && dx build --platform web {{web_features}} --debug-symbols false
	@just _worker-build "{{web_out_debug}}"
	@just _sync-dist "{{web_out_debug}}"

# Optimized frontend → dist/ (wasm-opt, hashed). Used by build-release.
ui-bundle: tailwind
	rm -rf "{{bundle_stage}}"
	cd "{{uidir}}" && dx bundle --platform web --profile wasm-release {{web_features}} --debug-symbols false --out-dir "{{bundle_stage}}"
	@just _worker-build "{{bundle_stage}}/public"
	@just _sync-dist "{{bundle_stage}}/public"

# Live dev: tailwind --watch + `dx serve` (hot reload) on :8000.
ui-watch:
	#!/usr/bin/env bash
	set -euo pipefail
	cd "{{uidir}}"
	# Compile once so dx's initial bundle has assets/tailwind.css (else 404 → SPA fallback).
	tailwindcss -i tailwind.css -o assets/tailwind.css
	tailwindcss -i tailwind.css -o assets/tailwind.css --watch &
	tw=$!
	# Build the worker once (dx serve can't); the poll below copies it into dx's public/.
	just _worker-build "{{cargo_target}}/worker-dev" || echo "warn: worker build failed; web worker disabled under dx serve"
	# dx serve rewrites public/ on rebuilds, so re-sync PWA root files + worker on a poll.
	dxpub="../../target/dx/halogen-ui/debug/web/public"
	( while true; do
	    if [ -d "$dxpub" ]; then
	      cp -ru pwa/. "$dxpub/" 2>/dev/null || true
	      cp -ru "{{cargo_target}}/worker-dev/worker" "$dxpub/" 2>/dev/null || true
	    fi
	    sleep 2
	  done ) &
	pwasync=$!
	trap 'kill $tw $pwasync 2>/dev/null || true' EXIT
	dx serve --platform web --port 8000 {{web_features}}

# ── Native frontend (desktop) ─────────────────────────────────────────────────
# Same webview UI: every native build runs tailwind first AND passes
# --no-default-features (default = web; leaving it on pulls the wasm renderer in).

# Live desktop dev run (webview, hot reload).
ui-desktop-serve: tailwind
	cd "{{uidir}}" && dx serve --platform desktop --no-default-features --features desktop

# Desktop build (debug by default; pass --release for a shippable binary).
ui-desktop-build *ARGS: tailwind
	cd "{{uidir}}" && dx build --platform desktop --no-default-features --features desktop {{ARGS}}

# CAUTION: dx stages into the SAME dir as a linux desktop release — don't interleave the two.
# Windows cross-build (x86_64-pc-windows-gnu via mingw-w64, `windows-release` profile).
ui-windows-build *ARGS: tailwind
	cd "{{uidir}}" && dx build --platform desktop --target x86_64-pc-windows-gnu --profile windows-release --no-default-features --features desktop {{ARGS}}

# Compile-check the native feature chains on the host target (assets/tailwind.css must exist).
check-native: _tailwind-ensure
	cargo check -p halogen-ui --no-default-features --features desktop

# ── Native iOS (ios-*): SwiftUI app + Rust core (crates/mobile-ffi) ──────────
# Pure SwiftUI over UniFFI bindings — no dioxus/webview. Pipeline: cargo
# staticlib → uniffi-bindgen swift → stage into ios/ → xcodegen → xcodebuild.

ios_dir := justfile_directory() / "ios"

# Swift Codable mirrors of the #[typeshare] wire DTOs (needs `cargo install typeshare-cli`).
ios-wire-types:
	typeshare --lang=swift --config-file typeshare.toml --output-file "{{ios_dir}}/Generated/WireTypes.swift" crates/wire crates/wire-meta

# Stage the debug or release Rust core, by name (ios-e2e's PROFILE dep).
_ios-core-for PROFILE:
	just {{ if PROFILE == "release" { "ios-core-release" } else { "ios-core" } }}

# Simulator Rust core + regenerated Swift bindings (host build first — bindgen reads the host cdylib).
ios-core: ios-wire-types
	cargo build -p halogen-mobile-ffi
	IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p halogen-mobile-ffi --target aarch64-apple-ios-sim
	cargo run -p halogen-mobile-ffi --features bindgen --bin uniffi-bindgen -- generate --library "{{cargo_target}}/debug/libhalogen_mobile.dylib" --language swift --out-dir "{{ios_dir}}/Generated" --no-format
	mkdir -p "{{ios_dir}}/Rust/lib/iphonesimulator" "{{ios_dir}}/Rust/include"
	cp "{{cargo_target}}/aarch64-apple-ios-sim/debug/libhalogen_mobile.a" "{{ios_dir}}/Rust/lib/iphonesimulator/"
	cp "{{ios_dir}}/Generated/halogen_mobileFFI.h" "{{ios_dir}}/Rust/include/"

# Device slice (real hardware; needs signing). Xcode picks a slice by $(PLATFORM_NAME).
ios-core-device:
	IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p halogen-mobile-ffi --target aarch64-apple-ios
	mkdir -p "{{ios_dir}}/Rust/lib/iphoneos"
	cp "{{cargo_target}}/aarch64-apple-ios/debug/libhalogen_mobile.a" "{{ios_dir}}/Rust/lib/iphoneos/"

# Regenerate Halogen.xcodeproj from project.yml and build for the simulator.
ios-build: ios-core
	cd "{{ios_dir}}" && xcodegen generate
	cd "{{ios_dir}}" && xcodebuild -project Halogen.xcodeproj -scheme Halogen -configuration Debug -destination 'generic/platform=iOS Simulator' -derivedDataPath build build

# Build + install + launch in a simulator (headless-friendly; streams the app console).
ios-run DEVICE="": ios-build
	#!/usr/bin/env bash
	set -euo pipefail
	device='{{DEVICE}}'
	if [ -z "$device" ]; then
	  device="$(xcrun simctl list devices available | sed -n 's/^ *\(iPhone [^(]*\)(.*/\1/p' | head -n1 | sed 's/ *$//')"
	  [ -n "$device" ] || { echo "error: no available iPhone simulator (xcodebuild -downloadPlatform iOS?)" >&2; exit 1; }
	fi
	echo "Using simulator: $device"
	xcrun simctl bootstatus "$device" -b
	open "$(xcode-select -p)/Applications/Simulator.app" 2>/dev/null \
	  || open -a Simulator 2>/dev/null \
	  || echo "warn: couldn't open the Simulator.app viewer — device is booted; open Simulator manually to see it"
	xcrun simctl install "$device" "{{ios_dir}}/build/Build/Products/Debug-iphonesimulator/Halogen.app"
	xcrun simctl launch --console "$device" org.fgsec.halogen

# Artifacts → data/ios/artifacts/e2e/; the default device spares 'iPhone 17' (manual testing).
# iOS e2e journeys: hermetic seeded server on :8099 + XCUITest suite, strictly serial.
ios-e2e DEVICE="iPhone 17 Pro" PROFILE="debug": (_ios-core-for PROFILE)
	#!/usr/bin/env bash
	set -euo pipefail
	out="{{justfile_directory()}}/data/ios/artifacts/e2e"
	rm -rf "$out" && mkdir -p "$out"
	e2etmp="$(mktemp -d)"
	mkdir -p "$e2etmp/media" "$e2etmp/public/feeds"
	cp "{{justfile_directory()}}/data/tests/"*.xml "$e2etmp/public/feeds/"
	cargo build -p halogen-server {{ if PROFILE == "release" { "--release --features dev-seed" } else { "" } }}
	"{{cargo_target}}/{{PROFILE}}/halogen-server" \
	  --db-path "$e2etmp/halogen.db" \
	  --media-root "$e2etmp/media" \
	  --log-file "$out/server.log" \
	  --enable-public-server --public-root "$e2etmp/public" \
	  --admin-username dev --admin-password dev \
	  --auth-token-secret dev-secret \
	  --listen-port 8099 \
	  --dev-use-mock-download \
	  --dev-seed-data &
	server_pid=$!
	vid_pid=""
	cleanup() {
	  [ -n "$vid_pid" ] && kill -INT "$vid_pid" 2>/dev/null && wait "$vid_pid" 2>/dev/null || true
	  kill "$server_pid" 2>/dev/null || true
	  rm -rf "$e2etmp"
	}
	trap cleanup EXIT
	for _ in $(seq 1 120); do
	  curl -sf -m 1 http://127.0.0.1:8099/healthz >/dev/null && break
	  sleep 0.5
	done
	curl -sf -m 2 http://127.0.0.1:8099/healthz >/dev/null || { echo "error: server never came up" >&2; exit 1; }
	cd "{{ios_dir}}" && xcodegen generate
	# Compute budget: exactly one simulator runs during e2e — shut down the rest first.
	xcrun simctl shutdown all 2>/dev/null || true
	# Best-effort run video: boot the device first so recording can attach.
	xcrun simctl bootstatus '{{DEVICE}}' -b || true
	(xcrun simctl io '{{DEVICE}}' recordVideo --force "$out/run.mov" & echo $! > "$e2etmp/vid.pid") 2>/dev/null || true
	vid_pid="$(cat "$e2etmp/vid.pid" 2>/dev/null || true)"
	# xcodebuild refuses an existing bundle (an aborted run can write one during teardown).
	rm -rf "$out/run.xcresult"
	set +e
	xcodebuild test \
	  -project "{{ios_dir}}/Halogen.xcodeproj" -scheme Halogen \
	  -destination 'platform=iOS Simulator,name={{DEVICE}}' \
	  -derivedDataPath "{{ios_dir}}/build" \
	  -resultBundlePath "$out/run.xcresult" \
	  TEST_RUNNER_HALOGEN_E2E_BASE=http://127.0.0.1:8099
	status=$?
	set -e
	# Capture the app's own log (network failures included) alongside the rest.
	app_container="$(xcrun simctl get_app_container '{{DEVICE}}' org.fgsec.halogen data 2>/dev/null || true)"
	[ -n "$app_container" ] && cp "$app_container/Library/Application Support/device-log.json" "$out/" 2>/dev/null || true
	echo "artifacts: $out"
	exit $status

# Committed on purpose — review artifacts; iPhone 17 Pro is an accepted App Store size.
# App Store screenshots from the e2e journeys → data/screenshots/ios/latest/.
ios-screenshots DEVICE="iPhone 17 Pro" PROFILE="debug": (ios-e2e DEVICE PROFILE)
	#!/usr/bin/env bash
	set -euo pipefail
	out="{{justfile_directory()}}/data/screenshots/ios/latest"
	rm -rf "$out" && mkdir -p "$out"
	tmp="$(mktemp -d)"
	trap 'rm -rf "$tmp"' EXIT
	xcrun xcresulttool export attachments \
	  --path "{{justfile_directory()}}/data/ios/artifacts/e2e/run.xcresult" \
	  --output-path "$tmp" >/dev/null
	python3 - "$tmp" "$out" <<'PY'
	import json, pathlib, re, shutil, sys
	tmp, out = map(pathlib.Path, sys.argv[1:3])
	manifest = json.load(open(tmp / "manifest.json"))
	tests = manifest if isinstance(manifest, list) else manifest.get("testAttachments", [])
	copied = 0
	for test in tests:
	    ident = test.get("testIdentifier") or "run"
	    # "EmbeddedJourneyTests-…" → embedded/, "RemoteJourneyTests-…" → remote/
	    m = re.match(r"([A-Za-z]+)JourneyTests", ident)
	    journey = m.group(1).lower() if m else re.sub(r"[^A-Za-z0-9]+", "-", ident).strip("-")
	    for att in test.get("attachments", []):
	        name = att.get("suggestedHumanReadableName") or ""
	        # Journey step snaps only — skip failure shots, recordings, dumps.
	        if not name.lower().endswith(".png") or name.startswith("MISSING"):
	            continue
	        step = name.split("_")[0]
	        dest = out / journey
	        dest.mkdir(parents=True, exist_ok=True)
	        shutil.copy(tmp / att["exportedFileName"], dest / f"{step}.png")
	        copied += 1
	print(f"{copied} screenshots -> {out}")
	PY

# ── Release / TestFlight (ios-*-release) ──────────────────────────────────────

# Optimized Rust core staged for device + simulator (overwrites debug staging; `just ios-core` reverts).
ios-core-release: ios-wire-types
	cargo build -p halogen-mobile-ffi --release
	IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p halogen-mobile-ffi --release --target aarch64-apple-ios-sim
	IPHONEOS_DEPLOYMENT_TARGET=17.0 cargo build -p halogen-mobile-ffi --release --target aarch64-apple-ios
	cargo run -p halogen-mobile-ffi --release --features bindgen --bin uniffi-bindgen -- generate --library "{{cargo_target}}/release/libhalogen_mobile.dylib" --language swift --out-dir "{{ios_dir}}/Generated" --no-format
	mkdir -p "{{ios_dir}}/Rust/lib/iphonesimulator" "{{ios_dir}}/Rust/lib/iphoneos" "{{ios_dir}}/Rust/include"
	cp "{{cargo_target}}/aarch64-apple-ios-sim/release/libhalogen_mobile.a" "{{ios_dir}}/Rust/lib/iphonesimulator/"
	cp "{{cargo_target}}/aarch64-apple-ios/release/libhalogen_mobile.a" "{{ios_dir}}/Rust/lib/iphoneos/"
	cp "{{ios_dir}}/Generated/halogen_mobileFFI.h" "{{ios_dir}}/Rust/include/"

# DEVELOPMENT_TEAM in the env → automatic signing; else unsigned (sign in Organizer).
# Release archive for TestFlight → target/ios-release/Halogen.xcarchive.
ios-archive: ios-core-release
	#!/usr/bin/env bash
	set -euo pipefail
	cd "{{ios_dir}}" && xcodegen generate
	out="{{justfile_directory()}}/target/ios-release"
	mkdir -p "$out"
	rm -rf "$out/Halogen.xcarchive"
	sign_args=(CODE_SIGNING_ALLOWED=NO)
	if [ -n "${DEVELOPMENT_TEAM:-}" ]; then
	  sign_args=(DEVELOPMENT_TEAM="$DEVELOPMENT_TEAM" CODE_SIGN_STYLE=Automatic CODE_SIGN_IDENTITY="Apple Development")
	fi
	# Marketing version = workspace version; build number = UTC minute stamp (monotonic).
	mv="$(sed -n '/^\[workspace.package\]/,/^\[/p' "{{justfile_directory()}}/Cargo.toml" | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/')"
	bn="$(date -u +%Y%m%d%H%M)"
	echo "archive: version ${mv} build ${bn}"
	xcodebuild -project "{{ios_dir}}/Halogen.xcodeproj" -scheme Halogen -configuration Release \
	  -destination 'generic/platform=iOS' \
	  -archivePath "$out/Halogen.xcarchive" \
	  MARKETING_VERSION="${mv}" CURRENT_PROJECT_VERSION="${bn}" \
	  archive "${sign_args[@]}"
	echo "archive: $out/Halogen.xcarchive"
	echo "next: Xcode Organizer (or xcodebuild -exportArchive + ExportOptions.plist) → App Store Connect → TestFlight"

# Signing happens host-side via tart-sign — the VM never sees the key (docs/TART.md).
# Stage the Release device .app at artifacts/release/ios/ for the host collector.
ios-collect: ios-archive
	rm -rf "{{justfile_directory()}}/artifacts/release/ios"
	mkdir -p "{{justfile_directory()}}/artifacts/release/ios"
	cp -R "{{justfile_directory()}}/target/ios-release/Halogen.xcarchive/Products/Applications/Halogen.app" "{{justfile_directory()}}/artifacts/release/ios/"
	@echo "staged: artifacts/release/ios/Halogen.app (host: scp into ci/tart/artifacts/<TART_TAG>/release/, then just tart-sign)"

# ── Native Android (android-*): Kotlin/Compose app + Rust core ───────────────
# Pipeline: typeshare kotlin → cargo-ndk .so (x86_64 + arm64-v8a) →
# uniffi-bindgen kotlin → gradle. Gradle bootstraps via the committed wrapper.

android_dir := justfile_directory() / "android"

# Kotlin mirrors of the #[typeshare] wire DTOs (needs `cargo install typeshare-cli`).
android-wire-types:
	mkdir -p "{{android_dir}}/generated/wire"
	typeshare --lang=kotlin --config-file typeshare.toml --output-file "{{android_dir}}/generated/wire/WireTypes.kt" crates/wire crates/wire-meta

# Rust core .so for both ABIs + regenerated Kotlin bindings (host build first —
# bindgen reads the host cdylib). PROFILE: debug|release.
android-core PROFILE="debug": android-wire-types
	cargo build -p halogen-mobile-ffi {{ if PROFILE == "release" { "--release" } else { "" } }}
	cargo ndk --platform 30 -t x86_64 -t arm64-v8a -o "{{android_dir}}/app/src/main/jniLibs" build -p halogen-mobile-ffi {{ if PROFILE == "release" { "--release" } else { "" } }}
	cargo run -p halogen-mobile-ffi --features bindgen --bin uniffi-bindgen -- generate --library "{{cargo_target}}/{{PROFILE}}/libhalogen_mobile.so" --language kotlin --out-dir "{{android_dir}}/generated/uniffi" --no-format

# Assemble the APK (debug|release). Version stamps mirror ios-archive.
android-build PROFILE="debug": (android-core PROFILE)
	#!/usr/bin/env bash
	set -euo pipefail
	version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "{{justfile_directory()}}/Cargo.toml" | head -n1)"
	# Minutes since epoch: monotonic like ios-archive's minute stamp, but
	# fits Android's Int32 versionCode (yymmddHHMM overflows it).
	code="$(( $(date -u +%s) / 60 ))"
	cd "{{android_dir}}"
	./gradlew ":app:assemble{{ if PROFILE == "release" { "Release" } else { "Debug" } }}" -PversionName="$version" -PversionCode="$code"

# Build the instrumentation-test APK (journey tests).
android-test-build:
	cd "{{android_dir}}" && ./gradlew :app:assembleDebugAndroidTest

# Stage the release APK at artifacts/release/android/.
android-apk-collect: (android-build "release")
	rm -rf "{{justfile_directory()}}/artifacts/release/android"
	mkdir -p "{{justfile_directory()}}/artifacts/release/android"
	cp "{{android_dir}}/app/build/outputs/apk/release/app-release.apk" "{{justfile_directory()}}/artifacts/release/android/halogen.apk"

# Remote-account journey on the droiddriver emulator against a seeded server
# running HERE (emucli reverse tunnel → 10.0.2.2:PORT); Android's ios-e2e.
# Needs DROIDDRIVER_URL/KEY + `emucli` >= 0.3.23 on PATH (https support).
android-e2e CLASS="org.fgsec.halogen.RemoteJourneyTest" PORT="8099":
	#!/usr/bin/env bash
	set -euo pipefail
	out="{{justfile_directory()}}/data/android/artifacts/e2e"
	rm -rf "$out" && mkdir -p "$out"
	e2etmp="$(mktemp -d)"
	mkdir -p "$e2etmp/media" "$e2etmp/public/feeds"
	cp "{{justfile_directory()}}/data/tests/"*.xml "$e2etmp/public/feeds/"
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
	# cwd must be the repo root: --dev-seed-data resolves its audio fixture
	# (data/tests/nasa-test-clip.mp3) relative to it.
	cd "{{justfile_directory()}}"
	"{{cargo_target}}/debug/halogen-server" \
	  --db-path "$e2etmp/halogen.db" \
	  --media-root "$e2etmp/media" \
	  --log-file "$out/server.log" \
	  --enable-public-server --public-root "$e2etmp/public" \
	  --admin-username dev --admin-password dev \
	  --auth-token-secret dev-secret \
	  --listen-port {{PORT}} \
	  --dev-use-mock-download \
	  --dev-seed-data &
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

# Committed on purpose — review artifacts, mirroring ios-screenshots: run both
# journeys and stage their step snaps → data/screenshots/android/latest/.
android-screenshots:
	#!/usr/bin/env bash
	set -euo pipefail
	out="{{justfile_directory()}}/data/screenshots/android/latest"
	rm -rf "$out" && mkdir -p "$out/remote" "$out/embedded"
	just android-e2e
	cp "{{justfile_directory()}}/data/android/artifacts/e2e/"*.png "$out/remote/"
	just android-e2e org.fgsec.halogen.EmbeddedJourneyTest
	cp "{{justfile_directory()}}/data/android/artifacts/e2e/"*.png "$out/embedded/"
	# Journey step snaps only — failure shots never belong in review artifacts.
	rm -f "$out"/*/*MISSING*.png
	echo "screenshots: $out"

# ── droiddriver helpers (dd-*): drive the remote Android emulator over HTTP ──
# Needs DROIDDRIVER_URL + DROIDDRIVER_KEY in the environment (CI secrets /
# ~/.droiddriver-key locally). The auth proxy intercepts every route — always
# send the key, even on /readyz.

_dd_curl := "curl -fsS -H \"X-Api-Key: $DROIDDRIVER_KEY\""

# Wait until the emulator guest reports booted.
dd-ready:
	{{_dd_curl}} --retry 60 --retry-delay 5 --retry-all-errors "$DROIDDRIVER_URL/readyz"

# Install an APK (default: the debug build) and optionally launch it.
dd-install APK="" LAUNCH="org.fgsec.halogen":
	#!/usr/bin/env bash
	set -euo pipefail
	apk='{{APK}}'
	[ -n "$apk" ] || apk="{{android_dir}}/app/build/outputs/apk/debug/app-debug.apk"
	{{_dd_curl}} --data-binary @"$apk" "$DROIDDRIVER_URL/apk?launch={{LAUNCH}}"

# Run instrumentation tests (package = the androidTest package).
dd-test PACKAGE="org.fgsec.halogen.test" CLASS="" TIMEOUT="1800":
	#!/usr/bin/env bash
	set -euo pipefail
	url="$DROIDDRIVER_URL/api/test?package={{PACKAGE}}&timeout={{TIMEOUT}}"
	[ -z "{{CLASS}}" ] || url="$url&class={{CLASS}}"
	{{_dd_curl}} -N -X POST "$url"

# Grab a PNG screenshot to FILE.
dd-screenshot FILE="artifacts/dd/screen.png":
	mkdir -p "$(dirname "{{FILE}}")"
	{{_dd_curl}} "$DROIDDRIVER_URL/screenshot.png" -o "{{FILE}}"
	@echo "wrote {{FILE}}"

# ── Dev server (dev-*) ────────────────────────────────────────────────────────

# Build the frontend first (ui-build/ui-watch).
# Server in dev config: devdata, dist/ served, dev/dev admin, mock downloads, seed data.
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

# ── Compile check (check-all) ─────────────────────────────────────────────────

# Compile-check what we ship: wasm frontend + native workspace (desktop checked elsewhere).
check-all:
	cargo check -p halogen-ui --no-default-features --features web --target wasm32-unknown-unknown
	cargo check -p halogen-api --target wasm32-unknown-unknown
	# tool-* dev binaries excluded — they pull the browser/e2e graph (built only via lighthouse/screenshots).
	cargo check --workspace --exclude halogen-tool-lighthouse --exclude halogen-tool-screenshot

# ── Build the embedded server binary (build-*) ────────────────────────────────

# Optimized server + embedded optimized frontend → target/release. PROFILE=release.
build-release: ui-bundle
	cargo build -p halogen-server --release --features embed-frontend

# The dev image IS the live site, so the frontend real users hit stays wasm-opt'd.
# Debug server + embedded RELEASE frontend → target/debug (the de-facto prod image).
build-dev: ui-build
	cargo build -p halogen-server --features embed-frontend

# ── Local image builds (image-*, docker) ──────────────────────────────────────
# FROM the toolchain image (TOOLCHAIN_IMAGE knob, default registry.<root>/…);
# export TOOLCHAIN_IMAGE=halogen-toolchain:local (after image-toolchain) for offline.

# Build-toolchain image → halogen-toolchain:local. BASE_IMAGE is required: the
# base is an in-cluster image, so this needs registry access (set CI_BASE_IMAGE).
image-toolchain:
	#!/usr/bin/env bash
	set -euo pipefail
	base="${CI_BASE_IMAGE:-}"
	[ -n "$base" ] || { echo "error: set CI_BASE_IMAGE (e.g. registry.<root>/ci-podman-dx-win:latest)" >&2; exit 1; }
	docker build -f ci/docker/toolchain/Dockerfile \
	  --build-arg BASE_IMAGE="$base" \
	  --build-arg NPM_REGISTRY="${npm_config_registry:-https://registry.npmjs.org/}" \
	  -t halogen-toolchain:local .

# Release image → halogen:release (= the v* CI image).
image-release:
	#!/usr/bin/env bash
	set -euo pipefail
	[ -n "{{toolchain_image}}" ] || { echo "error: set TOOLCHAIN_IMAGE or SERVICES_ROOT_DOMAIN (.env)" >&2; exit 1; }
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/')"
	docker build -f ci/docker/release/Dockerfile \
	  --build-arg TOOLCHAIN="{{toolchain_image}}" \
	  --build-arg PROFILE=release --build-arg VERSION="${version}" \
	  -t halogen:release .

# Debug image → halogen:dev (= the dev-* CI image; large binary).
image-dev:
	#!/usr/bin/env bash
	set -euo pipefail
	[ -n "{{toolchain_image}}" ] || { echo "error: set TOOLCHAIN_IMAGE or SERVICES_ROOT_DOMAIN (.env)" >&2; exit 1; }
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/')"
	docker build -f ci/docker/release/Dockerfile \
	  --build-arg TOOLCHAIN="{{toolchain_image}}" \
	  --build-arg PROFILE=dev --build-arg VERSION="${version}" \
	  -t halogen:dev .

# ── Tart macOS VM (tart-* — mac/iOS builds; docs/TART.md) ─────────────────────
# Config from .env at the repo root (copy .env.example); shell env overrides.

# One-time / on-toolchain-bump: packer-build the halogen-builder VM image.
tart-build-vm:
	ci/tart/tart-build-vm.sh

# All release artifacts for a git tag (iOS gate + app, then mac desktop) + sign. TART_TAG=vX.Y.Z overrides.
tart-release:
	ci/tart/build-releases.sh

# iOS release only: e2e gate + screenshots + unsigned Release device app.
tart-release-ios:
	ci/tart/build-release-ios.sh

# macOS desktop release only.
tart-release-desktops:
	ci/tart/build-release-desktops.sh

# Host-side signing of collected artifacts (no-ops with a notice until signing vars are set).
tart-sign *ARGS:
	ci/tart/sign-apple.sh {{ARGS}}

# Upload the signed iOS .ipa to App Store Connect / TestFlight (asks first; `-y` skips).
tart-upload *ARGS:
	ci/tart/upload-apple.sh {{ARGS}}

# One-time: persistent devboxvm clone joined to the tailnet (remote dev over ssh/VNC).
tart-devboxvm:
	ci/tart/tart-create-devboxvm.sh

# Delete leftover build-* VMs — never the builder image or devboxvm (`-n` = dry run).
tart-clean *ARGS:
	ci/tart/tart-clean.sh {{ARGS}}

# ── CI triggers (ci-* — push a tag → a workflow run) ──────────────────────────

# Push ci-toolchain-<ts> → toolchain.yml (rebuild + push the toolchain image).
ci-toolchain-rebuild:
	#!/usr/bin/env bash
	set -euo pipefail
	tag="ci-toolchain-$(date +%Y%m%d-%H%M%S)"
	git tag "${tag}"
	git push internal "${tag}"
	echo "Pushed ${tag} -> CI rebuilds the toolchain image"

fmt:
	cargo fmt --all

# mac/iOS ship separately via `just tart-release`; bump (version-bump-*) + commit first.
# Push v<version> → release.yml: image :<version> + :latest AND the Forgejo release with assets.
ci-tagged-release:
	#!/usr/bin/env bash
	set -euo pipefail
	echo "Formatting..."
	cargo fmt --all
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/')"
	tag="v${version}"
	# Tag must point at a clean, pushed commit — CI checks out the tagged ref. Cargo.lock
	# drift is exempt but does NOT ship: CI builds from the COMMITTED lock at the tag.
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
	echo "Pushed ${tag} -> CI builds & pushes halogen:${version} (+ :latest) and publishes the Forgejo release (server binary, flatpak/AppImage/bare ELF, windows zip)"

# Push dev-<ts> → release.yml dev flow (same tests, debug image :dev-<version> + :dev).
ci-dev-release:
	#!/usr/bin/env bash
	set -euo pipefail
	#if [ -n "$(git status --porcelain)" ]; then
	#  echo "error: working tree dirty; commit or stash before a dev release" >&2
	#  exit 1
	#fi
	version="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/')"
	tag="dev-$(date +%Y%m%d-%H%M%S)"
	git tag "${tag}"
	git push internal "${tag}"
	echo "Pushed ${tag} -> CI builds & pushes halogen:dev-${version} (+ :dev)"

# ── Version bump (version-bump-*) ─────────────────────────────────────────────
# Bump [workspace.package] in Cargo.toml, then cargo build (refresh lock) + commit + ci-tagged-release.

version-bump-bugfix: (_version-bump "patch")
version-bump-minor: (_version-bump "minor")
version-bump-major: (_version-bump "major")

# ── Tests (test-*) ────────────────────────────────────────────────────────────

# Every tier in order: unit → integ → ui → e2e (stops at first failure).
test-all: test-unit test-integ test-ui test-e2e

# Workspace unit tests (e2e/integ + the UI crate graph excluded — they run via their own tiers).
test-unit:
	cargo nextest run --workspace --exclude halogen-e2e --exclude halogen-integ \
	  --exclude halogen-ui --exclude halogen-ui-state \
	  --exclude halogen-ui-widgets --exclude halogen-ui-episode-list --exclude halogen-ui-views \
	  --exclude halogen-ui-icons \
	  --exclude halogen-ui-commands --exclude halogen-ui-forms --exclude halogen-ui-listview \
	  --exclude halogen-ui-toast --exclude halogen-ui-appstate --exclude halogen-ui-platform \
	  --exclude halogen-ui-idb \
	  --exclude halogen-ui-logging --exclude halogen-ui-config --exclude halogen-ui-svc-store \
	  --exclude halogen-ui-svc-media --exclude halogen-ui-svc-ws --exclude halogen-ui-cache-purge \
	  --exclude halogen-ui-svc-sync --exclude halogen-ui-svc-player --exclude halogen-ui-accounts \
	  --exclude halogen-tool-lighthouse --exclude halogen-tool-screenshot

# Tier-1 HTTP integration (real axum + SQLite, mocked RSS). Auto-includes any *_flow.rs.
test-integ:
	cargo nextest run -p halogen-integ --no-fail-fast

# Release wasm, not debug (debug risks CI OOM); concurrency capped in .config/nextest.toml.
# Tier-2 browser e2e: release frontend + headless Chrome (chromedriver or WEBDRIVER_URL).
test-e2e: ui-build
	cargo nextest run -p halogen-e2e --no-fail-fast --run-ignored ignored-only

# Needs chromedriver + Chrome; extra flags pass through (--base-url URL --user u --pass p).
# Web-Vitals walkthrough (CLS/blocking/INP/LCP per step) → target/lighthouse/report.{md,json}.
lighthouse *ARGS: ui-build
	cargo run -p halogen-tool-lighthouse {{ARGS}}

# Web-Vitals against an ALREADY-RUNNING instance (no wasm build); creds via args or LH_USER/LH_PASS.
lighthouse-ext url *ARGS:
	cargo run -p halogen-tool-lighthouse -- --base-url {{url}} {{ARGS}}

# Needs chromedriver + Chrome; extra flags pass through (--base-url skips build + seeding).
# Screenshot walkthrough of every screen/state → data/screenshots/web/<datetime>/ (+ latest/).
screenshots *ARGS: ui-build
	cargo run -p halogen-tool-screenshot {{ARGS}}

# ── Single-test variants (test-*-one) ─────────────────────────────────────────
# The arg is a nextest substring filter, e.g. `just test-integ-one media`.

# One unit test (same workspace selection as test-unit).
test-unit-one filter:
	cargo nextest run --workspace --exclude halogen-e2e --exclude halogen-integ --exclude halogen-ui --exclude halogen-tool-lighthouse --exclude halogen-tool-screenshot {{filter}}

# One integration test.
test-integ-one filter:
	cargo nextest run -p halogen-integ --no-fail-fast {{filter}}

# One browser e2e test (builds dist/ first, serial, runs #[ignore] tests).
test-e2e-one filter: ui-build
	cargo nextest run -p halogen-e2e --no-fail-fast --run-ignored ignored-only --test-threads=1 {{filter}}

# Doctests (nextest doesn't run them). No-op today (no doctests); guards future ones.
test-doc:
	cargo test --workspace --exclude halogen-e2e --exclude halogen-ui --exclude halogen-tool-lighthouse --exclude halogen-tool-screenshot --doc

test-api:
	cargo nextest run -p halogen-api

# Wire DTOs — run with `db` so the SeaORM-facing tests (DbValidationErrors, …) compile in.
test-wire:
	cargo nextest run -p halogen-wire --features db

# orm/fixture have no unit tests yet (covered by integ); add recipes when they grow some.

test-migrate:
	cargo nextest run -p halogen-migrate

# In-process server supervisor (Embedded Server mode) — each test boots a real server on loopback.
test-embedded:
	cargo nextest run -p halogen-embedded-server

test-utils:
	cargo nextest run -p halogen-utils

test-server:
	cargo nextest run -p halogen-server

# Compile-check the wasm-only sync Web Worker crate (its protocol tests live in ui-svc-sync).
test-ui-worker:
	cargo check -p halogen-ui-worker --target wasm32-unknown-unknown

# UI unit tests across the split UI crate graph, renderless/native (no wasm `web` renderer).
test-ui: test-ui-worker
	cargo nextest run --no-default-features \
	  -p halogen-ui -p halogen-ui-state -p halogen-ui-widgets \
	  -p halogen-ui-episode-list -p halogen-ui-views -p halogen-ui-icons \
	  -p halogen-ui-commands -p halogen-ui-forms -p halogen-ui-listview \
	  -p halogen-ui-toast -p halogen-ui-appstate -p halogen-ui-platform \
	  -p halogen-ui-idb \
	  -p halogen-ui-logging -p halogen-ui-config -p halogen-ui-svc-store \
	  -p halogen-ui-svc-media -p halogen-ui-svc-ws -p halogen-ui-cache-purge \
	  -p halogen-ui-svc-sync -p halogen-ui-svc-player -p halogen-ui-accounts
