# Add Vesper to your stack

Vesper is one container with a web UI, YAML configuration and SQLite storage. Add it to your existing Compose stack and reverse proxy; use your own paths, volumes and network names. No repository checkout, Rust or Node installation is needed.

**Authentication is optional. Without it, anyone who can reach Vesper has full administrator access.** Restrict access to your trusted network or use your proxy's authentication. Vesper still requires an HTTPS origin and a trusted reverse proxy.

## Container

Use `ghcr.io/cfbender/vesper:latest` (currently **linux/amd64**). `latest` follows successful main builds; `sha-<full 40-character commit SHA>` tags and image digests let you pin a build. If GHCR requires login, use `docker login ghcr.io` with a GitHub token that has `read:packages` and access to the package; enter it at the password prompt, never in your Compose file.

Merge this service into your stack, adapting the sample paths and proxy network:

```yaml
services:
  vesper:
    image: ghcr.io/cfbender/vesper:latest
    restart: unless-stopped
    user: "10001:10001" # any non-root UID:GID; see Storage and permissions
    read_only: true
    cap_drop: [ALL]
    security_opt: [no-new-privileges:true]
    tmpfs:
      - /tmp:size=16m,mode=1777,noexec,nosuid,nodev
    environment:
      VESPER_PUBLIC_URL: https://status.example.com
      VESPER_TRUSTED_PROXIES: 172.30.0.2/32 # replace with your proxy's address
    volumes:
      - ./vesper/config:/config
      - ./vesper/data:/data
      - ./vesper/secrets:/run/secrets:ro
    networks: [proxy]

networks:
  proxy:
    external: true # your existing proxy network
```

The image listens on **8080** and includes its own health check. Point your proxy at `http://vesper:8080` using your usual routing configuration. There is no host `ports:` mapping. For Traefik labels, optional authentication and a separate egress network, adapt the [reference Compose file](../deploy/compose.yaml).

### Storage and permissions

Host paths are entirely up to you; bind mounts, named volumes and your existing secret provisioning can all be used. These container paths are the defaults:

| Mount | Purpose | Required access for the container user |
| --- | --- | --- |
| `/config` | Authoritative YAML, including UI edits | Read/write, or read-only as below |
| `/data` | SQLite history, incidents, delivery state and uploaded icons | Read/write; persistent, local storage |
| `/run/secrets` | Optional credential files and private CA bundles | Read-only; mount only what Vesper needs |

The image runs as UID:GID `10001:10001` by default. Any other non-root UID:GID works too: set it with `user:`, or with `VESPER_UID` and `VESPER_GID` in `.env` for the [reference Compose file](../deploy/compose.yaml). For example, use `1000:1000` to keep the host files owned by your own account. Vesper doesn't need the user to exist in the image, so no PUID/PGID variables, entrypoint chown or permissions container is involved.

Create bind-mount directories before starting and give the chosen UID/GID the listed access. With named volumes, Docker copies the image's ownership (10001) into a new, empty volume, so fix ownership yourself if you run as another user. Do not run as root to work around permissions, and never mount the Docker socket.

For Git-managed or read-only configuration, mount `/config:ro` and set `VESPER_CONFIG_READ_ONLY: "true"`. The UI is then view-only: it hides adding and editing services, and Settings shows why configuration is read-only. A `/config` that the container's UID can't write is treated the same way. Otherwise UI edits write the same YAML files you manage on disk. `/data` must stay writable in either mode. See the [environment variable reference](../README.md#install-from-ghcr) if you need different container paths.

### Proxy and network

- Set `VESPER_PUBLIC_URL` to the exact external HTTPS origin, including a nonstandard port if used, with no URL path.
- Set `VESPER_TRUSTED_PROXIES` to the proxy's **immediate TCP peer address**, not client IPs or forwarding headers. Prefer stable `/32` or `/128` addresses. Trust a whole subnet only if it contains exclusively trusted proxies and Vesper; every trusted peer has administrator access. Don't allowlist an entire shared application network.
- Give Vesper outbound access to the targets you monitor and, if used, your notification services. An internal-only Docker network needs an additional egress network. The probe allowlist below is separate from proxy trust.
- Authentication is your choice: reuse your existing middleware, use proxy BasicAuth, or leave it off. Keep `VESPER_DEV_MODE` off in deployments. See [security and optional auth examples](security.md#optional-authentication-examples) for details.

## Initial configuration

Create `_index.yaml` in the directory mounted at `/config`. Replace the example network and target with yours:

```yaml
network:
  allowed_cidrs: [192.168.1.0/24]

machines:
  home:
    name: Home server
    services:
      jellyfin:
        name: Jellyfin
        checks:
          web:
            kind: http
            url: http://192.168.1.10:8096/health
```

HTTP checks expect status 200 by default; TCP and push checks are also supported. Active checks run every 30 seconds with a 5-second timeout, become Down after 3 consecutive failures and recover after 2 passes. These defaults can be overridden in YAML.

`network.allowed_cidrs` must cover the actual resolved destination addresses, including container networks when using container DNS names. Loopback, link-local and cloud metadata addresses are always denied; use a LAN/container address rather than `localhost`. For private TLS certificates, mount a CA bundle in `/run/secrets` and set `network.private_ca_file` to its filename. TLS verification stays enabled.

Start with one file or split machines, services and checks into directories later. See the [configuration reference](../crates/core/CONFIGURATION.md) and [multi-machine examples](../examples/config) for layouts, secret references and additional checks. Invalid reloads keep the last valid configuration active.

## Start and check

Run these from your stack directory (add your usual Compose flags if needed):

```sh
docker compose pull vesper
docker compose run --rm --no-deps vesper validate
docker compose up -d --no-deps vesper
docker compose logs --tail 50 vesper
```

If validation fails, fix the reported configuration error before starting or updating the service.

Open the configured HTTPS URL. Confirm the inventory appears, Settings shows valid configuration, and active checks have recent results after about a minute. Push checks need a heartbeat first. A healthy container means the server is alive, not that every check or notification is working.

Check your intended access boundary too: excluded clients must not reach management routes; if authentication is enabled, logged-out requests to both the dashboard and `/api/snapshot` must be denied or redirected. [Security](security.md#reverse-proxy-and-authentication-contract) covers the full proxy contract.

## Optional notifications and push checks

**Notifications:** pick one or more channel kinds from [notifications](notifications.md). For Telegram, create a bot with BotFather, message it or add it to your group, and obtain the chat ID. Mount credential files such as `telegram-bot-token` and `telegram-chat-id` under `/run/secrets`, readable by the container's UID/GID but not other users. Add this to the root configuration:

```yaml
notifications:
  dashboard_url: https://status.example.com
  channels:
    telegram:
      kind: telegram
      bot_token: {file: telegram-bot-token}
      chat_id: {file: telegram-chat-id}
```

A self-hosted service on your LAN, such as Gotify or ntfy, must be inside `network.allowed_cidrs`; public services are reachable without listing them. Use **Settings → Notifications → Send test message** on each channel and confirm it arrives. API acceptance alone is not delivery confirmation. Secrets can also use `{env: NAME}` references to variables you supply to the container; never put literal credentials in YAML or paste them into chat.

**Push checks:** configure `kind: push`, a `token: {file: my-push-token}` reference, and your `expected_interval_seconds` and `grace_seconds`. Send `{"healthy": true}` or `{"healthy": false}` to `POST /api/heartbeat/<machine>/<service>/<check>` with the per-check token in the `Authorization: Bearer …` header, never in the URL. A missed cadence plus grace opens an incident; a healthy heartbeat recovers it.

If your proxy authenticates users, bypass interactive auth **only for that exact POST route**, preserving its bearer header. Apply rate and body limits as in the [reference Compose file](../deploy/compose.yaml); a missing or wrong token must receive Vesper's JSON 401, not a login page. [Producer scripts](../examples/producers) and [systemd examples](../examples/systemd) cover Docker, Proxmox and scheduled jobs; adapt their paths and schedules to your hosts. Vesper cannot detect its own complete failure: use an independent monitor, optionally with the [watchdog example](../examples/watchdog).

## Updates and backups

Back up your YAML, persistent data, deployment settings and secrets (encrypted). For consistent SQLite backups, stop Vesper while copying `/data`, or use SQLite's online `.backup`; never copy just `vesper.db` while it is running because committed data may still be in its WAL file.

To update, select a published GHCR tag/digest, keep the previous digest and a pre-upgrade backup, then repeat the pull, validate and start commands above. For rollback, restore the previous image and, if the database format changed, its matching backup. Recheck inventory, recent results and notifications. If replacing another monitor, keep it running until you're satisfied with Vesper's coverage.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Container exits or is unhealthy | Logs, valid YAML, HTTPS `VESPER_PUBLIC_URL`, and writable `/data` for the container's UID (10001 unless overridden) |
| Dashboard/API returns 403 | Actual proxy peer address against `VESPER_TRUSTED_PROXIES`; proxy network selection |
| Browser edits return 403 | Browser origin matches `VESPER_PUBLIC_URL`; proxy preserves Origin and CSRF headers |
| Checks say destination not allowed | Resolved target address against `network.allowed_cidrs`; no loopback/link-local targets |
| Config changes do not activate | Settings' validation error; the last valid configuration remains active |
| Heartbeat is rejected or gets a login page | Check ID, per-check token, exact POST route and proxy auth exception |

For an agent-assisted install, have the agent adapt these settings to your existing stack, storage and proxy conventions. If this is your first stack, choose those conventions together rather than assuming a particular host layout. Shared proxy, DNS and firewall changes should be agreed with you before applying them.
