# syntax=docker/dockerfile:1
# Embedded-frontend server; PROFILE=release|dev, cached dependencies, TOOLCHAIN base.
# Build: docker build -f ci/docker/release/Dockerfile -t halogen:release .
# Run: docker run --rm -p 8080:8080 -e HALOGEN_AUTH_TOKEN_SECRET=... halogen:release

# REQUIRED build-arg (no default — keeps the internal registry host out of the
# repo): registry.${SERVICES_ROOT_DOMAIN}/halogen-toolchain:latest in CI, or
# halogen-toolchain:local after `just docker-build-toolchain`.
ARG TOOLCHAIN
FROM ${TOOLCHAIN} AS chef
WORKDIR /src

############################################
# Plan — capture the dependency graph only #
############################################
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

############################################
# Build — cook deps (cached), then build   #
############################################
FROM chef AS builder

# Server profile: release or dev. The frontend always uses wasm-release.
ARG PROFILE=release

# Disable the repo's sccache default: the cooked layer IS the cache, and
# sccache's server can't start under buildah's chroot isolation.
ENV RUSTC_WRAPPER=""

# Cook server dependencies in PROFILE and frontend/worker in wasm-release.
# Copy Cargo configuration first to preserve registry and linker settings.
COPY .cargo .cargo
COPY --from=planner /src/recipe.json recipe.json
RUN REL=$([ "$PROFILE" = dev ] && echo "" || echo "--release"); \
    cargo chef cook $REL -p halogen-server --features embed-frontend \
        --recipe-path recipe.json \
 && cargo chef cook --profile wasm-release --target wasm32-unknown-unknown \
        -p halogen-webui --no-default-features --features web \
        --recipe-path recipe.json \
 && cargo chef cook --profile wasm-release --target wasm32-unknown-unknown \
        -p halogen-webui-worker --recipe-path recipe.json

# Real source + the actual build (profile-selected).
COPY . .
RUN cd webui/app && npm ci
RUN if [ "$PROFILE" = dev ]; then \
        just ui-build && cargo build -p halogen-server --features embed-frontend \
        && cp target/debug/halogen-server /usr/local/bin/halogen-server ; \
    else \
        just build-release && cp target/release/halogen-server /usr/local/bin/halogen-server ; \
    fi

############################
# Runtime — slim           #
############################
# Runtime glibc must match the toolchain's Fedora, or the binary won't start.
FROM docker.io/library/fedora:44 AS runtime

# ca-certificates ships in the base; reqwest (rustls) uses the system roots.
# No OpenSSL; SQLite is bundled into the binary.
RUN useradd --system --create-home --uid 10001 halogen

COPY --from=builder /usr/local/bin/halogen-server /usr/local/bin/halogen-server

# OCI version label (defaults to the workspace version; release tooling passes it).
ARG VERSION=0.1.0
LABEL org.opencontainers.image.title="halogen-server" \
      org.opencontainers.image.description="Halogen Podcast Server" \
      org.opencontainers.image.version="${VERSION}"

# Persistent state (SQLite DB + downloaded media) lives under /data.
RUN mkdir -p /data/media && chown -R halogen:halogen /data
USER halogen
VOLUME ["/data"]

# Set a stable HALOGEN_AUTH_TOKEN_SECRET. Startup rejects a missing secret.
ENV HALOGEN_LISTEN_ADDRESS=0.0.0.0 \
    HALOGEN_LISTEN_PORT=8080 \
    HALOGEN_DB_PATH=/data/halogen.db \
    HALOGEN_MEDIA_ROOT=/data/media \
    HALOGEN_LOG_LEVEL=info

EXPOSE 8080
ENTRYPOINT ["halogen-server"]
