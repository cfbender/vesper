# Vesper

Vesper is a homelab status monitor: one Rust/Axum process with a bundled React UI, SQLite history, YAML as the only configuration authority, and Telegram notifications. It runs HTTP(S), TCP and push-heartbeat checks across your machines and groups them into services and incidents. It runs behind an authenticating reverse proxy. Vesper has no accounts of its own, so every user the proxy admits is an administrator.

- **Install:** [docs/installation.md](docs/installation.md) is a step-by-step runbook, written so an agent can follow it. It covers YAML layout, secrets, validation, Traefik and Authelia, producers, Telegram, drills, the watchdog, upgrades and backups.
- **Security model:** [docs/security.md](docs/security.md)
- **Architecture and API contract:** [docs/architecture.md](docs/architecture.md)
- **Examples:** [examples/](examples): a complete multi-host inventory, invalid layouts, Docker and Proxmox push producers, systemd units and an independent watchdog.
- **Reference deployment:** [deploy/compose.yaml](deploy/compose.yaml): one non-root, read-only container on a private network behind Traefik, with no published port.

## What v1 does and does not do

It checks HTTP status and body substrings, TCP connects, and push heartbeats with per-check bearer tokens. Docker and Proxmox health arrive as pushed heartbeats from small producers on those hosts. Vesper never mounts the Docker socket or runs remote commands. It has no built-in login, no ICMP, DNS or certificate checks, and no high availability. It cannot report its own outage, so pair it with the [watchdog](examples/watchdog) and an independent monitor such as Uptime Kuma.

## Build

```sh
docker build -t vesper:$(git rev-parse --short HEAD) .
deploy/smoke-test.sh vesper:$(git rev-parse --short HEAD)   # optional; needs docker and jq
```

The image runs `vesper serve` by default. Other commands: `vesper validate` checks the configuration under `VESPER_CONFIG_DIR`, `vesper export` prints the assembled configuration as one block YAML document with secret references only, and `vesper healthcheck` probes the local liveness endpoint.

| Variable | Container default | Meaning |
| --- | --- | --- |
| `VESPER_PUBLIC_URL` | (required) | Exact `https://` origin users reach, with no path. Used for Origin/CSRF checks and links |
| `VESPER_TRUSTED_PROXIES` | (none) | Comma-separated CIDRs of the authenticating proxy. Other peers get 403 |
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

Checks before sending a change: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, plus the web package's own scripts. CI ([.github/workflows/ci.yml](.github/workflows/ci.yml)) runs those, shellcheck, unit verification and the container smoke test. It never publishes images or deploys.

## License

MIT
