# syntax=docker/dockerfile:1
# Full test matrix image (run-tests.sh: fmt → checks → unit/ui/doc/integ/e2e);
# cargo-chef cooks deps, toolchain via the TOOLCHAIN build-arg.
# Build: docker build -f ci/docker/testing/Dockerfile -t halogen:testing .
# Run:   docker run --rm halogen:testing [just test-integ]

# REQUIRED build-arg (no default — keeps the internal registry host out of the
# repo): registry.${SERVICES_ROOT_DOMAIN}/halogen-toolchain:latest in CI, or
# halogen-toolchain:local after `just image-toolchain`.
ARG TOOLCHAIN
FROM ${TOOLCHAIN} AS chef
WORKDIR /workspace

############################################
# Plan — capture the dependency graph only #
############################################
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

############################################
# Build — cook deps + bake a runnable image#
############################################
FROM chef AS builder

# OCI version label (defaults to the workspace version; CI passes it).
ARG VERSION=0.1.0
LABEL org.opencontainers.image.title="halogen-testing" \
      org.opencontainers.image.description="Halogen full test matrix" \
      org.opencontainers.image.version="${VERSION}"

# Strip debuginfo: DWARF is >half the target/, and this image bakes the cooked
# target + recompiles into the runner's size-limited graphroot. Via CARGO_PROFILE_*
# (not RUSTFLAGS — that'd clobber the mold flag). Tests don't need it.
ENV CARGO_PROFILE_DEV_DEBUG=0 \
    CARGO_PROFILE_TEST_DEBUG=0

# Disable the repo's sccache default: the cooked layer IS the cache, and
# sccache's server can't start under buildah's chroot isolation.
ENV RUSTC_WRAPPER=""

# Two cook passes: native workspace with --tests (covers the test-support
# graph via dev-deps) + the release wasm frontend (matches `just ui-build`).
COPY .cargo .cargo
COPY --from=planner /workspace/recipe.json recipe.json
RUN cargo chef cook --workspace --tests --recipe-path recipe.json \
 && cargo chef cook --release --target wasm32-unknown-unknown \
        -p halogen-ui --no-default-features --features web \
        --recipe-path recipe.json

# Real source.
COPY . .
# npm ci in a subshell so its `cd` doesn't leak into `install` (which resolves
# run-tests.sh from WORKDIR).
RUN (cd crates/ui && npm ci) \
    && install -m 0755 ci/docker/testing/run-tests.sh /usr/local/bin/run-tests

# Default: run the whole matrix. Override with any command (e.g. `just test-e2e`).
CMD ["run-tests"]
