# syntax=docker/dockerfile:1.7
#
# One runtime image: the `vesper` server binary plus the built web UI.
#
#   docker build -t vesper:$(git rev-parse --short HEAD) .
#
# Runtime contract (see docs/installation.md and deploy/compose.yaml):
#   - runs as UID:GID 10001:10001, works with a read-only root filesystem,
#     all capabilities dropped and no-new-privileges
#   - writable state only in /data (SQLite) and, optionally, /config (YAML)
#   - secrets mounted read-only at /run/secrets; no Docker socket unless
#     optional label discovery is enabled (docs/discovery.md)
#   - listens on 0.0.0.0:8080 inside a private network; publish no host port

ARG RUST_VERSION=1.93.1
ARG NODE_VERSION=24
ARG DEBIAN_RELEASE=trixie

# --- Web UI -----------------------------------------------------------------
FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm \
    npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build && test -f dist/index.html

# --- Server -----------------------------------------------------------------
FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS server
WORKDIR /src
# The image already ships the pinned toolchain; pinning RUSTUP_TOOLCHAIN keeps
# rustup from downloading the extra rustfmt/clippy components that
# rust-toolchain.toml lists for development.
ARG RUST_VERSION
ENV RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_TERM_COLOR=never \
    CARGO_INCREMENTAL=0
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    rustc --version \
    && cargo build --release --locked --bin vesper \
    && install -m 0755 target/release/vesper /usr/local/bin/vesper

# --- Runtime ----------------------------------------------------------------
FROM debian:${DEBIAN_RELEASE}-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 vesper \
    && useradd --system --uid 10001 --gid 10001 --no-create-home \
       --home-dir /nonexistent --shell /usr/sbin/nologin vesper \
    && install -d -o 10001 -g 10001 -m 0750 /data /config \
    && install -d -o root -g 10001 -m 0750 /run/secrets
COPY --from=server /usr/local/bin/vesper /usr/local/bin/vesper
COPY --from=web /src/web/dist /app/web

ENV VESPER_CONFIG_DIR=/config \
    VESPER_DATA_DIR=/data \
    VESPER_SECRETS_DIR=/run/secrets \
    VESPER_STATIC_DIR=/app/web \
    VESPER_LISTEN=0.0.0.0:8080

LABEL org.opencontainers.image.title="Vesper" \
      org.opencontainers.image.description="Homelab status monitor" \
      org.opencontainers.image.source="https://github.com/cfbender/vesper" \
      org.opencontainers.image.licenses="MIT"

USER 10001:10001
WORKDIR /data
EXPOSE 8080
# `vesper healthcheck` probes the local /healthz endpoint before any config
# parsing, so it needs no curl in the image.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/vesper", "healthcheck"]
# tini reaps children and forwards SIGTERM so `docker stop` is prompt.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/vesper"]
CMD ["serve"]
