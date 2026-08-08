# syntax=docker/dockerfile:1
# Thin DEV image: the debug halogen-server (compiled outside docker by
# dev-release.yml, frontend embedded) COPY'd onto fedora. Context must contain
# ./halogen-server. Build: docker build -f ci/docker/dev/Dockerfile -t halogen:dev .

# Runtime glibc must match the toolchain's — that now builds on ci-podman-dx
# (Fedora 44), so an older-glibc base → "GLIBC_2.xx not found". Keep in step
# with the ci-podman-base Fedora version.
FROM docker.io/library/fedora:44

# ca-certificates ships in the base; reqwest (rustls) uses the system roots.
RUN useradd --system --create-home --uid 10001 halogen

COPY halogen-server /usr/local/bin/halogen-server

ARG VERSION=0.0.0-dev
LABEL org.opencontainers.image.title="halogen-server (dev)" \
      org.opencontainers.image.description="Halogen Podcast Server — debug build" \
      org.opencontainers.image.version="${VERSION}"

RUN mkdir -p /data/media && chown -R halogen:halogen /data
USER halogen
VOLUME ["/data"]

# Defaults to debug logging; set a stable HALOGEN_AUTH_TOKEN_SECRET in deploys
# (the default is randomized per boot → invalidates JWTs on restart).
ENV HALOGEN_LISTEN_ADDRESS=0.0.0.0 \
    HALOGEN_LISTEN_PORT=8080 \
    HALOGEN_DB_PATH=/data/halogen.db \
    HALOGEN_MEDIA_ROOT=/data/media \
    HALOGEN_LOG_LEVEL=debug

EXPOSE 8080
ENTRYPOINT ["halogen-server"]
