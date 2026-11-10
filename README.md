# Vesper

Vesper is a homelab status monitor: one Rust/Axum process with a bundled React UI, SQLite history, YAML as the only configuration authority, and Telegram notifications. It runs HTTP(S), TCP and push-heartbeat checks across your machines and groups them into services and incidents. It runs behind a reverse proxy with optional authentication. **Vesper has no built-in login: without external authentication, anyone who can reach it has full administrator access.**

- **Install:** [Add Vesper to your stack](docs/installation.md): a Compose service, storage requirements, initial configuration and startup checks. Adapt it to your existing paths and proxy.
- **Security model:** [docs/security.md](docs/security.md)
- **Configuration reference:** [crates/core/CONFIGURATION.md](crates/core/CONFIGURATION.md), including supported layouts and save guarantees.
- **Architecture and API contract:** [docs/architecture.md](docs/architecture.md)
- **Examples:** [examples/](examples): a complete multi-host inventory, invalid layouts, Docker and Proxmox push producers, systemd units and an independent watchdog.
- **Reference deployment:** [deploy/compose.yaml](deploy/compose.yaml): one non-root, read-only container on a private network behind Traefik, with no published port.

## What v1 does and does not do

It checks HTTP status and body substrings, TCP connects, and push heartbeats with per-check bearer tokens. Docker and Proxmox health arrive as pushed heartbeats from small producers on those hosts. Vesper never mounts the Docker socket or runs remote commands. It has no built-in login, no ICMP, DNS or certificate checks, and no high availability. It cannot report its own outage, so pair it with the [watchdog](examples/watchdog) and an independent monitor such as Uptime Kuma.

## Install from GHCR

Use the published **`ghcr.io/cfbender/vesper`** image (currently `linux/amd64`), not a local build. Successful `main` CI publishes `latest` and `sha-<full 40-character commit SHA>` tags. `latest` follows the most recently published main build; pin its digest for repeatable deployments. There are no release-version tags yet.

If the package is private, first run `docker login ghcr.io -u YOUR_GITHUB_USERNAME` and enter a GitHub personal access token (classic) with `read:packages` and access to the package at the password prompt. Do not put the token in a command or `.env`.

```sh
docker pull ghcr.io/cfbender/vesper:latest
docker image inspect --format '{{index .RepoDigests 0}}' ghcr.io/cfbender/vesper:latest
```

Use the image tag or digest in your Compose service's `image:` field. The [installation guide](docs/installation.md) has a minimal service to merge into your stack; the [Traefik example](deploy/compose.yaml) includes routing labels and uses `VESPER_IMAGE` for interpolation. Do not publish the backend port directly.

The image runs `vesper serve` by default. Other commands: `vesper validate` checks the configuration under `VESPER_CONFIG_DIR`, `vesper export` prints the assembled configuration as one block YAML document with secret references only, and `vesper healthcheck` probes the local liveness endpoint.

| Variable | Container default | Meaning |
| --- | --- | --- |
| `VESPER_PUBLIC_URL` | (required) | Exact `https://` origin users reach, with no path. Used for Origin/CSRF checks and links |
| `VESPER_TRUSTED_PROXIES` | (none) | Comma-separated CIDRs of the reverse proxy. Other peers get 403 |
| `VESPER_CONFIG_DIR` | `/config` | YAML root |
| `VESPER_CONFIG_READ_ONLY` | `false` | `true` makes the UI draft/test/export-only |
| `VESPER_DATA_DIR` | `/data` | SQLite (`vesper.db`) |
| `VESPER_SECRETS_DIR` | `/run/secrets` | Files for `{file: name}` references and the private CA |
| `VESPER_STATIC_DIR` | `/app/web` | Built web UI |
| `VESPER_LISTEN` | `0.0.0.0:8080` | Listen address |
| `VESPER_DEV_MODE` | `false` | Local development only. Never set it in a deployment |

## Local development

```sh
npm --prefix web ci && npm --prefix web run build
mkdir -p /tmp/vesper-dev/data /tmp/vesper-dev/secrets
VESPER_DEV_MODE=true VESPER_LISTEN=127.0.0.1:8080 \
VESPER_CONFIG_DIR=$PWD/examples/config VESPER_CONFIG_READ_ONLY=true \
VESPER_DATA_DIR=/tmp/vesper-dev/data VESPER_SECRETS_DIR=/tmp/vesper-dev/secrets \
VESPER_STATIC_DIR=$PWD/web/dist \
  cargo run --bin vesper -- serve
```

The example inventory points at `192.168.x` addresses and secret files that do not exist locally. Expect its checks to fail with explanatory messages until you point them at your own targets.

For development-only container builds and smoke tests:

```sh
docker build -t vesper:dev .
deploy/smoke-test.sh vesper:dev
```

Checks before sending a change: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, plus the web package's own scripts. [CI](.github/workflows/ci.yml) runs those, shellcheck, unit verification and the container smoke test. Only successful pushes to `main` in this repository call the [GHCR publisher](.github/workflows/publish-image.yml), which smoke-tests its exact image before pushing with the built-in `GITHUB_TOKEN`. Pull requests and other branches never publish. No workflow deploys.

## License

MIT
