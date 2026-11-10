# Vesper security model

Vesper is a single-tenant homelab monitor. It has no accounts of its own. Its safety depends on the deployment boundary described here. Treat any deviation from it as a security change.

## Trust boundaries

```text
 browser ──HTTPS──▶ Traefik ──forward-auth──▶ Authelia
                      │  (admitted users only, every route except one)
                      │
 push producer ─HTTPS─┤  POST /api/heartbeat/<m>/<s>/<c>  (no Authelia; bearer token)
                      ▼
            vesper-ingress (internal, Traefik + Vesper only)
                      ▼
                 Vesper :8080 ──egress bridge──▶ LAN targets in allowed_cidrs, api.telegram.org
```

- **The authentication proxy is the only access control.** Vesper has no users, passwords, OIDC or sessions. Every request that Traefik forwards from an admitted user is an administrator request: it can edit monitoring YAML (when writable), silence, acknowledge, upload icons and send Telegram tests. Restrict the Authelia rule to the people who should have that power ([deploy/authelia-access-control.yml](../deploy/authelia-access-control.yml)).
- **Trusted peers.** Vesper serves the dashboard and API only to TCP peers inside `VESPER_TRUSTED_PROXIES`. Peers anywhere else get 403. The reference deployment trusts only the internal `vesper-ingress` network, which Traefik alone joins. Anyone who can attach a container to that network can act as an administrator. That is already a Docker administrator, who is root-equivalent on the host. `VESPER_DEV_MODE=true` relaxes this check for local development only. Never set it in a deployment.
- **No published port.** The container publishes nothing on the host. The only way in is through Traefik.
- **`/healthz`** answers any direct peer with liveness only: no inventory, no versions, no credentials. Through Traefik it stays behind Authelia.

## The heartbeat exception

Exactly one route skips Authelia: `POST` to `/api/heartbeat/<machine>/<service>/<check>`, with three ID segments of `A-Z a-z 0-9 _ -`. The Traefik rule uses an anchored `PathRegexp` and `Method(POST)`, at a higher priority than the main router. It adds a per-source rate limit and a 1 KiB body limit. Other methods, depths, prefixes and encoded variants fall through to the authenticated router. [installation.md](installation.md) phase 10 lists the checks that prove this.

On that route, Vesper:

- requires `Authorization: Bearer <token>`, where the token is the check's own push token. A token is valid for exactly one check;
- reveals nothing on failure. A wrong token, a missing token and an unknown check all give the same 401 `{"error": …}`;
- returns no inventory, needs no CSRF token, and never takes a secret from the URL.

Push producers ([examples/producers](../examples/producers)) send the token only in the header, fed to curl on stdin. The token is never in an argument, a URL or a log. Producers refuse plain-HTTP URLs.

## Browser protections

- Every management mutation requires the `X-Vesper-CSRF` token from `GET /api/csrf`, and an `Origin` header exactly equal to `VESPER_PUBLIC_URL`. That URL must be an `https` origin without a path. The token is random per process and is not a cookie or a session. CSRF protection is not authentication: Authelia still guards these endpoints.
- No permissive CORS. Monitored response bodies are never rendered as HTML, and probe evidence is sanitized and bounded.
- Icon uploads accept PNG, JPEG or WebP only, up to 2 MiB and 2048×2048 pixels. They are re-encoded to PNG of at most 256 px. User SVG is rejected; only bundled, trusted SVG is served.

## Secrets

- YAML holds only references, `{file: name}` (from `VESPER_SECRETS_DIR`, `/run/secrets`) or `{env: NAME}`. Literal values are rejected wherever a secret is expected.
- Secret files are confined to the secrets directory: one path segment, symlinks only within the root, regular UTF-8 files of at most 64 KiB. Mount that directory read-only, owned by root with group 10001 and mode 0440 or 0750.
- Resolved values never appear in API responses, `vesper export`, error messages, logs or Telegram messages. Configuration exports keep the references.
- Environment values are visible through `docker inspect`. Prefer files for credentials.
- Push tokens are per check. Generate them with `openssl rand -hex 32` and rotate them by replacing both copies.

## Outbound network policy

Active checks are a server-side request feature, so they are constrained:

- `network.allowed_cidrs` is deny-by-default and file-controlled. The UI cannot edit it, and draft tests obey the saved policy.
- Destinations are resolved, validated and then connected to by address, including every redirect hop. Loopback, link-local, cloud metadata, multicast, unspecified and IPv4-mapped IPv6 forms are always denied.
- Redirects are off by default and never carry secret headers across origins. URL userinfo is rejected. No ambient HTTP proxy is used.
- TLS verification is always on. A private CA is added with `network.private_ca_file`, never by disabling verification.
- Time, response size and concurrency are bounded.

## Container hardening

| Control | Reference setting |
| --- | --- |
| User | UID:GID 10001:10001 (image default and `user:` in compose) |
| Root filesystem | `read_only: true`. Writable mounts: `/data` (SQLite), `/config` only in writable mode, and a small `noexec` tmpfs at `/tmp` |
| Capabilities | `cap_drop: [ALL]`, `no-new-privileges:true` |
| Resources | `pids_limit: 256`, `mem_limit: 512m`, rotated JSON logs |
| Process | `tini` as PID 1, forwarding signals; `vesper healthcheck` for the Docker health check |
| Docker socket | Never mounted. Vesper runs no shell or remote commands |
| Image | Multi-stage build; the runtime holds the `vesper` binary, the web assets, CA certificates and `tini` only |

`deploy/smoke-test.sh` asserts these settings at runtime: UID, `CapEff`, `NoNewPrivs`, read-only root and config, and no port bindings.

## Producers and the watchdog

- **Docker:** the producer reads container state through the local Docker socket. Membership in the `docker` group is root-equivalent, so the script is root-owned and runs from a hardened systemd unit as a dedicated user. It receives its token through `LoadCredential`, and its only network action is an outbound HTTPS POST.
- **Proxmox:** a privilege-separated API token with only the read-only `PVEAuditor` role, running under `DynamicUser=yes`. No root, SSH or `pvesh`.
- **Watchdog:** it posts Vesper's own watchdog heartbeat and reports to Uptime Kuma. Kuma's push API carries its token in the URL by design. The URL lives in a root-only credential file and is passed to curl on stdin.

## Known limits

- Anyone admitted by Authelia is a full administrator. Vesper has no user identity of its own, so use Authelia's and Traefik's logs to attribute the actions in Vesper's audit log.
- An attacker who holds a check's push token can mark that one check healthy or unhealthy. Rotate the token if it is exposed.
- Monitoring from one vantage point cannot tell a target outage from a path outage. Telegram API acceptance is not delivery proof.
- Vesper cannot detect its own outage. Run the independent watchdog.

## Reporting a vulnerability

Report security problems privately through GitHub's "Report a vulnerability" (security advisories) on [cfbender/vesper](https://github.com/cfbender/vesper/security), not in a public issue.
