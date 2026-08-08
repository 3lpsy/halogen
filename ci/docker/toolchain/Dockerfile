# syntax=docker/dockerfile:1
# Shared build toolchain: release + testing `FROM` this (built once/pushed).
# On ci-podman-dx-win (Fedora): rust stable + wasm32 + windows-gnu/mingw,
# wasm-bindgen, sccache/nextest/just, node, CLIs, and the crates gate are
# inherited; only halogen's own desktop/packaging lane is added here.

# No default: CI passes the in-cluster base derived from the root-domain var.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}
WORKDIR /src

# Desktop (dioxus/webkit) dev stack + e2e chromium + xz for asset tarballs.
# TLS is rustls; SQLite bundled — no OpenSSL.
RUN dnf -y install --setopt=install_weak_deps=False \
        chromium chromedriver xz \
        webkit2gtk4.1-devel gtk3-devel libsoup3-devel \
        libayatana-appindicator-gtk3-devel librsvg2-devel libxdo-devel \
    && dnf clean all

RUN curl -fsSL -o /usr/local/bin/tailwindcss \
        https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-x64 \
    && chmod +x /usr/local/bin/tailwindcss

# Release packaging (flatpak bundle + windows zip); flathub runtimes live on
# /cache (setup-flatpak.sh), not baked. No flatpak-builder: build-flatpak.sh
# uses plain build-init/export, since bwrap can't nest in the rootless runner.
RUN dnf -y install --setopt=install_weak_deps=False \
        zip flatpak \
    && dnf clean all

# AppImage: appimagetool + runtime baked so releases don't download at
# package time; both upstreams publish only `continuous`.
RUN dnf -y install --setopt=install_weak_deps=False \
        file desktop-file-utils binutils \
    && dnf clean all \
    && curl -fsSL -o /usr/local/bin/appimagetool \
        https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage \
    && chmod +x /usr/local/bin/appimagetool \
    && mkdir -p /usr/local/lib/appimage \
    && curl -fsSL -o /usr/local/lib/appimage/runtime-x86_64 \
        https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64

# npm routing (the cargo gate is already baked in the base). Printed so a
# changed value never hides behind a stale cached layer.
RUN echo "baked: npm_config_registry=${npm_config_registry}" \
 && echo "baked: PIP_INDEX_URL=${PIP_INDEX_URL} CHILLED_PROXY_URL=${CHILLED_PROXY_URL}" \
 && (cat "${CARGO_HOME}/config.toml" 2>/dev/null || echo "cargo: public crates.io")

# ── Android app lane ─────────────────────────────────────────────────────────
# Mirrors ci-podman-android layer for layer: that image is ci-podman-base + this
# lane, and a Dockerfile cannot FROM both it and ci-podman-dx-win. Keep the pins
# in step with it, and with android/app/build.gradle.kts.

# aarch64 = devices/release; x86_64 = the droiddriver emulator (instrumented
# tests load x86_64 jniLibs). The NDK below carries the cross clang.
RUN rustup target add aarch64-linux-android x86_64-linux-android

# cargo-ndk: per-ABI cdylibs for gradle's jniLibs; typeshare-cli: Kotlin wire
# DTO mirrors. Through the base's gate; uniffi-bindgen stays per-project.
RUN CARGO_BUILD_JOBS=2 cargo install --locked cargo-ndk typeshare-cli \
    && rm -rf "${CARGO_HOME}/registry" "${CARGO_HOME}/git"

# JDK 25: the only LTS Fedora 44 still packages. AGP 9 runs on it.
RUN dnf -y install --setopt=install_weak_deps=False java-25-openjdk-devel \
    && dnf clean all
ENV JAVA_HOME=/usr/lib/jvm/java-25-openjdk

# ANDROID_API/BUILD_TOOLS track compileSdk; NDK is what cargo-ndk cross-links to.
ARG CMDLINE_TOOLS_BUILD=13114758
ARG ANDROID_API=36
ARG ANDROID_BUILD_TOOLS=36.0.0
ARG ANDROID_NDK=28.2.13676358

# Both spellings: gradle reads ANDROID_HOME, older tooling ANDROID_SDK_ROOT,
# cargo-ndk ANDROID_NDK_HOME.
ENV ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk
ENV ANDROID_NDK_HOME=${ANDROID_HOME}/ndk/${ANDROID_NDK} \
    PATH=${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools:${PATH}

# platform-tools also gives adb, though droiddriver runs go over its HTTP API.
RUN curl -fsSL "https://dl.google.com/android/repository/commandlinetools-linux-${CMDLINE_TOOLS_BUILD}_latest.zip" \
      -o /tmp/cmdline-tools.zip \
    && mkdir -p "${ANDROID_HOME}/cmdline-tools" \
    && unzip -q /tmp/cmdline-tools.zip -d "${ANDROID_HOME}/cmdline-tools" \
    && mv "${ANDROID_HOME}/cmdline-tools/cmdline-tools" "${ANDROID_HOME}/cmdline-tools/latest" \
    && rm -f /tmp/cmdline-tools.zip \
    && yes | sdkmanager --licenses > /dev/null \
    && sdkmanager \
         "platform-tools" \
         "platforms;android-${ANDROID_API}" \
         "build-tools;${ANDROID_BUILD_TOOLS}" \
         "ndk;${ANDROID_NDK}" \
    && rm -rf "${ANDROID_HOME}/.temp"

# Maven, so the base's ~/.m2/settings.xml mirrors are usable directly. Upstream
# tarball, not the rpm: the rpm drags its own JDK next to the one above.
ARG MAVEN_VERSION=3.9.16
ARG MAVEN_SHA512=831a8591fe20c8243b1dbe7d71e3244f31d1665b0804b2e825e38cbbe5ce0cafb8338851f90780735568773e0a6cd07bbec107cda0b896b008b861075358b6f6
RUN curl -fsSL -o /tmp/maven.tgz \
      "https://archive.apache.org/dist/maven/maven-3/${MAVEN_VERSION}/binaries/apache-maven-${MAVEN_VERSION}-bin.tar.gz" \
    && echo "${MAVEN_SHA512}  /tmp/maven.tgz" | sha512sum -c - \
    && tar -xzf /tmp/maven.tgz -C /opt \
    && ln -s "/opt/apache-maven-${MAVEN_VERSION}/bin/mvn" /usr/local/bin/mvn \
    && rm -f /tmp/maven.tgz \
    && mvn --version

# System Gradle for wrapper-less builds; android/ has a wrapper and ignores it.
# Pinned to the wrapper's version so the seeded cache below matches.
ARG GRADLE_VERSION=9.6.1
ARG GRADLE_SHA256=9c0f7faeeb306cb14e4279a3e084ca6b596894089a0638e68a07c945a32c9e14
RUN curl -fsSL -o /tmp/gradle.zip \
      "https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-bin.zip" \
    && echo "${GRADLE_SHA256}  /tmp/gradle.zip" | sha256sum -c - \
    && unzip -q /tmp/gradle.zip -d /opt \
    && ln -s "/opt/gradle-${GRADLE_VERSION}/bin/gradle" /usr/local/bin/gradle \
    && rm -f /tmp/gradle.zip \
    && gradle --version

# Gradle reads init scripts from $GRADLE_USER_HOME/init.d, so the home is fixed
# for the gate below to land anywhere findable.
ENV GRADLE_USER_HOME=/root/.gradle

# Gradle ignores ~/.m2/settings.xml, so the base's Maven mirrors never reach a
# Gradle build — this script repoints Central, the Plugin Portal and Google
# Maven at the proxy mounts using the base's $CHILLED_PROXY_URL. Vendored
# verbatim from the proxy's examples/gradle/; re-copy it, don't fork it.
COPY ci/docker/toolchain/chilled-proxy.init.gradle /etc/gradle/init.d/chilled-proxy.gradle
RUN mkdir -p "${GRADLE_USER_HOME}/init.d" \
 && cp /etc/gradle/init.d/chilled-proxy.gradle "${GRADLE_USER_HOME}/init.d/" \
 && echo "baked: gradle gate -> ${CHILLED_PROXY_URL:-<ungated>}"

# A job that repoints GRADLE_USER_HOME at the runner's /cache PVC leaves init.d
# behind — and the gate with it, with only an --info warning. Re-seeds whichever
# home is current; run it right after exporting GRADLE_USER_HOME.
RUN printf '%s\n' \
      '#!/bin/sh' \
      '# Copy the chilled-proxy gate into the active GRADLE_USER_HOME.' \
      'set -eu' \
      'target="${GRADLE_USER_HOME:-$HOME/.gradle}/init.d"' \
      'mkdir -p "$target"' \
      'cp /etc/gradle/init.d/chilled-proxy.gradle "$target/"' \
    > /usr/local/bin/chilled-gradle-init \
 && chmod 0755 /usr/local/bin/chilled-gradle-init

# Seed the wrapper's cache slot (base36(md5(distributionUrl))) with the
# verified distribution so a GRADLE_VERSION-pinned gradlew downloads nothing.
RUN set -eu; \
    url="https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-bin.zip"; \
    hash=$(python3 -c "import hashlib,sys;n=int.from_bytes(hashlib.md5(sys.argv[1].encode()).digest(),'big');d='0123456789abcdefghijklmnopqrstuvwxyz';print(''.join(reversed([d[(n//36**i)%36] for i in range(25)])).lstrip('0') or '0')" "$url"); \
    target="${GRADLE_USER_HOME}/wrapper/dists/gradle-${GRADLE_VERSION}-bin/$hash"; \
    mkdir -p "$target"; \
    ln -sfn "/opt/gradle-${GRADLE_VERSION}" "$target/gradle-${GRADLE_VERSION}"; \
    touch "$target/gradle-${GRADLE_VERSION}-bin.zip.ok"; \
    echo "baked: wrapper cache $target -> /opt/gradle-${GRADLE_VERSION}"

# dx MUST match the dioxus crate version (0.7.9); cargo-chef feeds release/
# testing's dep-cache stages. Through the base's deps-proxy crates gate.
RUN cargo install --locked dioxus-cli@0.7.9 cargo-chef \
    && rm -rf "${CARGO_HOME}/registry" "${CARGO_HOME}/git"

# CHROME_BIN deliberately unset: crates/e2e probes PATH (chromium-browser
# first) only when it's empty, and a wrong value is passed through verbatim.
ENV CHROMEDRIVER=/usr/bin/chromedriver

# Flathub runtimes live on /cache (setup-flatpak.sh), not baked.
ENV FLATPAK_USER_DIR=/cache/flatpak
