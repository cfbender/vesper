# Installing Vesper: an agent runbook

This runbook takes an operator, or an agent working for one, from a clean Docker host to a verified Vesper deployment. Vesper needs an authenticating reverse proxy, not a particular authentication provider. The concrete reference path uses Traefik v3 with your chosen auth middleware; phase 8 covers proxy-native BasicAuth and the suggested Authelia SSO/MFA option. Other proxies can satisfy the same [security contract](security.md#reverse-proxy-and-authentication-contract), but their configuration is not supplied here. If comparing with an existing Uptime Kuma installation, keep it running. Each phase names the commands to run, the result to expect and what to record. Do the phases in order. A phase is done only when its **Record** items are filled in from real output.

Read [security.md](security.md) first. The layout and field rules in this runbook follow [architecture.md](architecture.md) and `crates/core/src/model.rs`.

## Rules for the agent

- **Never print, paste or log secret values.** That covers bot tokens, push tokens, API tokens, chat IDs, Kuma push URLs and header values. Create secret files with the commands below, which never echo them. When the user must supply a secret, have them type it into a terminal prompt (`read -rs`), not into the chat.
- **Report only what you observed.** Copy decisive output verbatim. If you could not run a step, write "not verified" and the reason. Never write "Telegram works" unless a person confirmed that the test message arrived (phase 12).
- **Change nothing outside the scope you were given.** Do not stop, reconfigure or delete Uptime Kuma or its monitors. Do not publish Vesper's port, mount the Docker socket into Vesper, widen the scoped heartbeat authentication exception, or run Vesper as root. Ask before changing the user's shared proxy or authentication configuration, firewall or DNS.
- **Stop and ask** when a fact in phase 1 is unknown, when a check below gives an unexpected result, or when a step would affect other services.

## 0. Scope and limitations

Vesper v1 does the following:

- It runs **HTTP(S)** checks, with an expected status and an optional body substring, **TCP connect** checks, and **push heartbeats** (`POST /api/heartbeat/<machine>/<service>/<check>` with a per-check bearer token).
- It applies a failure, recovery and staleness policy to each check. It keeps incidents, history, acknowledgements and silences in SQLite, and sends Telegram outage and recovery notifications.
- It treats YAML files as the only configuration authority. The UI edits those same files when the config is writable.

It does not do the following:

- **No native Docker or Proxmox polling.** Vesper never touches the Docker socket, SSH or remote command execution. Docker and Proxmox health arrive as pushed heartbeats from small producers on those hosts (phase 11).
- **No ICMP ping, DNS-record or certificate-expiry checks**, and no other check types.
- **No built-in login, users, roles, OIDC or sessions.** Authentication and administrator authorization belong to the external proxy/auth layer. Authelia is optional. Everyone admitted to Vesper's management interface is a full administrator.
- **One instance, one vantage point.** Vesper sees the network from its container's egress network only. It is not highly available, and it cannot report its own outage; that is the job of the independent watchdog (phase 14).
- **Telegram is the only notifier.** The Bot API accepting a message is not a read receipt, and network ambiguity can cause duplicate sends.
- **Dependency grouping is configured, not inferred.** Only a service's own `depends_on` list creates a parent dependency. `host_service` marks which service probes the machine itself; it does not make the machine's other services depend on it. A correlated failure is not a proven root cause.

### Monitoring semantics

These are the behaviours the runbook's checks rely on. Use them to judge results.

- **Thresholds.** The defaults are a 30 s interval, a 5 s timeout, Down after 3 consecutive failures and Healthy after 2 consecutive passes. A check is Suspect from its first failure, and Recovering from its first pass after Down. A failed recovery resets the pass count and does not open a second incident. Runs never overlap.
- **Unknown is not failure.** After a restart or any gap in execution, checks show unknown or stale. Vesper does not invent failed samples for the gap. Pause stops execution, and a paused service is not counted as healthy.
- **Push checks.** One missed `expected_interval_seconds + grace_seconds` deadline opens an incident. One healthy heartbeat recovers it. An explicit `{"healthy": false}` makes the check Down immediately.
- **Incidents.** There is one incident per service, aggregating its checks. To recover, every enabled check must be fresh and healthy. Removing a service from the configuration closes its open incident, and sends a CLOSED notification if the outage notification was delivered.
- **Dependencies.** When a service's `depends_on` parent has a confirmed failure, the child's notification is grouped under the parent. The child's checks keep running and are rechecked when the parent recovers.
- **Acknowledge, silence, maintenance.** Acknowledging is awareness only, and it stops the opt-in reminders for that incident. Silences and maintenance windows suppress notifications, not execution. Weekly windows are UTC. A recovery that happens while a service is paused, silenced or in maintenance waits. When the suppression ends, still-open problems are notified, and recoveries go only to recipients who received the outage. An incident that was suppressed from start to finish stays history-only. `notifications: false` mutes a service's outages and recoveries.
- **Telegram.** There is one recipient chat. An outage and its recovery are each sent once per incident; reminders are opt-in through `reminder_seconds`. Retries honour `retry_after` and send a current summary instead of a backlog. If the Bot API rejects delivery with 400, 401, 403 or 404, Vesper stops sending until the configuration revision changes or an explicit test succeeds. Changing the chat ID means the old chat never receives recoveries for outages it was sent. Messages contain names, times, durations and the dashboard link, never probe responses or secrets.
- **Retention.** SQLite keeps the last 500 samples per check, and the snapshot shows the last 60 per check. Incidents, incident events and the audit log are never pruned. The snapshot lists every open incident plus the 100 most recently resolved, each with its last 100 events.

## 1. Facts to collect before starting

Ask the user for anything you cannot read from the host. Record every item.

| Fact | Example |
| --- | --- |
| Public origin (exact `https://host`, no path) | `https://status.example.com` |
| Docker host and repository checkout path | `docker1`, `/srv/vesper/src` |
| Traefik container name, TLS entrypoint, certificate resolver | `traefik`, `websecure`, `letsencrypt` |
| Authentication middleware and allowed administrators | `vesper-auth@file` (BasicAuth), or `authelia@file` with group `vesper-admins` |
| A free private /28 for the ingress network | `172.31.250.0/28` |
| LAN CIDRs Vesper may probe | `192.168.10.0/24, 192.168.20.0/24` |
| Private CA bundle (if internal services use one) | `/etc/ssl/homelab-root.pem` |
| Machines, services and checks to monitor (from Kuma's monitor list) | see phase 6 |
| Hosts for push producers, the watchdog host, the Kuma URL | `pve1`, `nas`, `https://kuma.example.com` |
| Telegram chat to notify | private chat or group |
| Whether the UI may edit YAML (writable) or YAML is Git-managed (read-only) | phase 4 |

Check the host:

```sh
docker version --format '{{.Server.Version}}'   # 24+ expected
docker compose version                          # v2.24+ (env_file `required:`)
docker inspect -f '{{.Config.Image}}' traefik   # Traefik v3.x
```

**Record:** the table above, the Docker and Compose versions, and the Traefik image.

## 2. Pull the published image

Install exclusively from `ghcr.io/cfbender/vesper`; no Rust, Node or local image build is required. The published platform is currently **linux/amd64**. Check `docker info --format '{{.Architecture}}'` before continuing; ARM hosts are not supported by this image yet.

Successful `main` CI publishes `latest` and `sha-<full 40-character commit SHA>`. Pull requests, feature branches and Git tags do not publish images. `latest` is a rolling main build, not a versioned release. Choose a reviewed commit tag when needed, then pin the pulled digest so later publishes cannot change an installed version.

For a private package, run `docker login ghcr.io -u YOUR_GITHUB_USERNAME` as the same account that will run Docker below. Enter a GitHub personal access token (classic) with `read:packages` and package access at the password prompt (authorize SSO if required). Never put it in a command, configuration file or chat. Package visibility is managed in GitHub; publishing does not make a private package public.

```sh
set -e
requested=ghcr.io/cfbender/vesper:latest   # or ghcr.io/cfbender/vesper:sha-<full-commit-sha>
docker pull "$requested"
image=$(docker image inspect --format '{{index .RepoDigests 0}}' "$requested")
revision=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' "$image")
docker image inspect -f '{{.Config.User}} {{.Config.Healthcheck.Test}}' "$image"
```

Expected: the pull succeeds, `image` is `ghcr.io/cfbender/vesper@sha256:…`, `revision` is the full source commit, and inspect prints `10001:10001 [CMD /usr/local/bin/vesper healthcheck]`. The image contains the server, production web UI, CA certificates and `tini`. If a pull fails, check package access and the successful CI publish job; do not substitute a local build.

Fetch matching deployment files, examples and producer scripts from the repository (authenticate Git separately if the repository is private). This checkout is for installation assets, not building:

```sh
sudo install -d -m 0755 /srv/vesper
sudo git clone https://github.com/cfbender/vesper /srv/vesper/src   # skip if already cloned
sudo git -C /srv/vesper/src fetch origin "$revision"
sudo git -C /srv/vesper/src checkout --detach "$revision"
cd /srv/vesper/src
```

Optional: run `deploy/smoke-test.sh "$image"` on this pulled image. It needs `jq` and pulls `curlimages/curl`.

**Record:** the source commit, requested tag, pinned image digest, inspect output, and smoke-test summary if run.

## 3. Prepare the host directories

```sh
sudo install -d -m 0755 /srv/vesper
sudo cp /srv/vesper/src/deploy/compose.yaml /srv/vesper/compose.yaml
sudo cp /srv/vesper/src/deploy/env.example /srv/vesper/.env
sudo sed -i "s|^VESPER_IMAGE=.*|VESPER_IMAGE=$image|" /srv/vesper/.env
sudo install -m 0600 -o root -g root /srv/vesper/src/deploy/vesper.env.example /srv/vesper/vesper.env
sudo install -d -m 0750 -o 10001 -g 10001 /srv/vesper/data
sudo install -d -m 0750 -o root -g 10001 /srv/vesper/secrets
```

Edit `/srv/vesper/compose.yaml` and change every line marked `CHANGE`: the origin in `VESPER_PUBLIC_URL` and both `Host()` rules, the ingress CIDR in `VESPER_TRUSTED_PROXIES`, the entrypoints and the certificate resolver. Set `VESPER_AUTH_MIDDLEWARE` in `/srv/vesper/.env` to the existing authentication middleware you choose in phase 8. Compose deliberately rejects an empty value; do not remove the middleware label to get past this check. Do not add `ports:`.

`/data` holds `vesper.db` and its `-wal` and `-shm` files. Keep it on local disk, not NFS or SMB.

## 4. Writable or read-only configuration

YAML in `/srv/vesper/config` is authoritative in both modes. Pick one mode and record it.

**Writable** (the UI can save edits). Vesper edits only the files that own changed fields, preserving unrelated values and comments. Unsafe edits become export-only. It checks the whole configuration revision again before replacing files. A non-cooperating external editor can still race that final check, and a crash during a multi-file edit can leave a partial change: file replacements are atomic, but the whole tree is not transactional. For strict Git-controlled changes, use read-only mode. See the [configuration reference](../crates/core/CONFIGURATION.md) for exact guarantees and supported layouts.

```sh
sudo install -d -m 0750 -o 10001 -g 10001 /srv/vesper/config
# later, after writing files: sudo chown -R 10001:10001 /srv/vesper/config
```

If the directory is also a Git checkout, commit the UI's changes from the host regularly. Vesper does not run Git.

**Read-only** (Git-managed, with changes made by pull request). Mount the directory read-only and force read-only mode, so the UI offers draft, test and export only:

```yaml
    environment:
      VESPER_CONFIG_READ_ONLY: "true"
    volumes:
      - ./config:/config:ro
```

The files only need to be readable by UID 10001: `chmod -R o+rX` or group 10001. Vesper polls the directory every few seconds. It activates only a changed revision that validates, and it keeps the last valid configuration (showing the error) otherwise. `POST /api/config/reload` activates a change immediately.

**Record:** the mode, the ownership and permissions (`ls -ln /srv/vesper`), and whether a Git remote backs the config.

## 5. Secrets

YAML never contains secret values. Every credential is a reference that Vesper resolves only when it needs the value:

| Reference | Resolves to | Use for |
| --- | --- | --- |
| `{file: name}` | `/run/secrets/name` (host `/srv/vesper/secrets/name`) | tokens, API keys, header values (preferred) |
| `{env: NAME}` | the container environment variable `NAME`, from `/srv/vesper/vesper.env` | low-sensitivity identifiers such as the Telegram chat ID |

The rules, as enforced by `vesper-core`:

- File names are a single path segment of `A-Z a-z 0-9 _ - .`, at most 128 bytes, and do not start with `.` or `-`. There are no subdirectories.
- A file may be a symlink only if it resolves inside the secrets directory. It must be a regular UTF-8 file of at most 64 KiB and must not be empty. One trailing newline is removed.
- Environment variable names match `[A-Za-z_][A-Za-z0-9_]*`.
- Environment values are visible to anyone who can run `docker inspect`. Files are visible only to root and UID/GID 10001.

Create file secrets without echoing them. Make the files group-readable by 10001:

```sh
# Random push token for check docker1/host/containers
sudo sh -c 'umask 027; openssl rand -hex 32 > /srv/vesper/secrets/push-docker1-host-containers'
# A value the user supplies: the user types it at the hidden prompt
read -rsp 'Paperless header value: ' v; printf '%s\n' "$v" | sudo sh -c 'umask 027; cat > /srv/vesper/secrets/paperless-api-authorization'; unset v
sudo sh -c 'chown root:10001 /srv/vesper/secrets/* && chmod 0440 /srv/vesper/secrets/*'   # glob inside sudo: the directory is not readable by you
```

Header secrets hold the complete header value, for example `Token abc…` or `Bearer abc…`.

**Private CA.** Put the PEM bundle in the secrets directory and name it in `network.private_ca_file`. It follows the same file-name rules:

```sh
sudo install -m 0440 -o root -g 10001 /etc/ssl/homelab-root.pem /srv/vesper/secrets/homelab-ca.pem
```

TLS verification is always on. The certificate must cover the host name or IP address in the check URL.

**Rotation.** For a push token, write the new value to both Vesper's secret file and the producer's credential file, then send a heartbeat and confirm it is accepted (phase 11). A heartbeat sent between the two writes is rejected. After rotating only the Telegram bot token file, run the explicit Telegram test (phase 12). A rotation that changes no YAML does not change the configuration revision, so if delivery was stopped by a rejection, only a successful test resumes it.

**Record:** `sudo ls -ln /srv/vesper/secrets` (names, owners and modes only) and the env variable names in `vesper.env`, not their values.

## 6. Write the inventory

Start from [`examples/config`](../examples/config), a complete four-machine example with 10 services and 13 checks. Copy it, then replace its contents with the user's real inventory:

```sh
sudo cp -r /srv/vesper/src/examples/config/. /srv/vesper/config/
sudo chown -R 10001:10001 /srv/vesper/config   # writable mode; read-only mode: sudo chmod -R o+rX /srv/vesper/config
```

### Layout rules

- `_index.yaml` holds the fields of the directory that contains it. The root `_index.yaml` holds `defaults`, `network` and `telegram`.
- `name.yaml` defines key `name`. `name/` defines key `name` as a mapping. A file never repeats its own key.
- The root `machines/` directory holds the machines. A machine is either `machines/<id>.yaml` (everything inline) or `machines/<id>/_index.yaml` plus `machines/<id>/services/<service>.yaml`. A service can split further into `services/<service>/_index.yaml` plus `services/<service>/checks/<check>.yaml`.
- IDs (machine, service and check keys or file names) use only ASCII letters, digits, `-` and `_`. Full IDs are `machine/service` and `machine/service/check`.
- Scalars and lists stay YAML values. There is no deep merge and no last-file-wins.

The example uses every form:

```text
examples/config/
├── _index.yaml                          defaults, network, telegram
└── machines/
    ├── router.yaml                      whole machine in one file
    ├── nas.yaml                         one file, incl. a paused service and a daily push check
    ├── pve1/
    │   ├── _index.yaml                  machine fields
    │   └── services/host.yaml           HTTPS check (private CA) + Proxmox push check
    └── docker1/
        ├── _index.yaml
        └── services/
            ├── host.yaml                TCP + Docker push check
            ├── paperless.yaml           HTTPS with a secret header
            ├── vesper.yaml              watchdog push check (phase 14)
            └── jellyfin/
                ├── _index.yaml          service fields + weekly maintenance
                └── checks/
                    ├── health.yaml      HTTP status + body substring
                    └── port.yaml        TCP
```

### Collisions Vesper rejects

Each case below is a runnable example under [`examples/invalid`](../examples/invalid). Each must fail validation:

- **File/directory collision:** `machines/nas.yaml` and `machines/nas/` both exist.
- **Index/child (inline/sibling) collision:** `machines/docker1/_index.yaml` has a `services:` key while `machines/docker1/services/` exists. The same applies to `checks:` in a service `_index.yaml` next to `checks/`, and to `machines:` in the root `_index.yaml`.
- **Duplicate definitions:** a repeated YAML key, or the same service defined twice.
- **Unknown fields:** for example `interval` instead of `interval_seconds`. Typos never fall back to defaults silently.
- **Literal credentials** in `headers`, `token`, `bot_token` or `chat_id`.
- **Invalid IDs**, symlinked YAML, files that escape the root, and excessive depth or size.

Vesper also refuses YAML edits from the UI when they would flatten or lose a split layout. Export the service and edit the files by hand instead.

### Check fields

| Kind | Required | Optional |
| --- | --- | --- |
| `http` | `url` (no `user:pass@`) | `expected_status` (200), `body_contains`, `headers` (secret refs), `follow_redirects` (false) |
| `tcp` | `host`, `port` | |
| `push` | `token` (secret ref) | `expected_interval_seconds` (300), `grace_seconds` (60) |

All kinds accept `name` and `enabled` (true). Active kinds accept the policy overrides `interval_seconds`, `timeout_seconds`, `failure_threshold`, `recovery_threshold` and `stale_after_seconds`. Services accept `name`, `icon`, `description`, `paused`, `notifications` (true), `depends_on` (full service IDs), `maintenance` (RFC3339 `start`/`end`, optional `repeat_weekly`, UTC) and `checks`. Machines accept `name`, `site`, `description`, `host_service` and `services`.

Icons: leave `icon` unset (`server`), or pick a bundled ID from the editor or `GET /api/icons`. Uploads accept PNG, JPEG or WebP up to 2 MiB and 2048×2048, re-encoded to at most 256 px PNG. User SVG is rejected.

### Network allowlist

`network.allowed_cidrs` is deny-by-default. List only the LAN ranges the checks need. Vesper resolves each destination, including every redirect hop, and connects only to an allowed address. Loopback, link-local, cloud metadata, multicast and unspecified addresses, and IPv4-mapped IPv6 forms of them, stay denied even when listed. `localhost` and `127.0.0.1` therefore never work. To check a service on the Docker host itself, use the host's LAN address and put it in an allowed CIDR. Vesper uses no ambient HTTP proxy. Also confirm that the host firewall (`DOCKER-USER` chain, UFW) lets the egress bridge reach those ranges.

### Converting from Uptime Kuma

List every Kuma monitor and map it to a machine/service/check. Record monitors with no v1 equivalent, such as ping, DNS, certificate expiry or Docker container monitors, and propose a TCP, HTTP or push replacement for each. Keep Kuma's monitors running (phase 15).

**Record:** the final tree (`find /srv/vesper/config -type f | sort`), the Kuma-to-Vesper mapping, and the monitors left unmapped with the reason.

## 7. Validate and export

Run the CLI as root (it reads the root-only `vesper.env`), with the same image, mounts and hardening as the service, and no network:

```sh
vesper_cli() {
  docker run --rm --network none --user 10001:10001 --read-only --cap-drop ALL \
    --security-opt no-new-privileges:true --env-file /srv/vesper/vesper.env \
    -v /srv/vesper/config:/config:ro -v /srv/vesper/secrets:/run/secrets:ro \
    "$(sed -n 's/^VESPER_IMAGE=//p' /srv/vesper/.env)" "$@"
}
vesper_cli validate; echo "exit=$?"
vesper_cli export > /tmp/vesper-export.yaml
docker run --rm -i mikefarah/yq:4 '{"machines": (.machines | length),
  "services": ([.machines[].services[]] | length),
  "checks": ([.machines[].services[].checks[]] | length)}' < /tmp/vesper-export.yaml
```

`validate` exits 0 for a valid tree. Otherwise it exits nonzero with a safe explanation on stderr. `export` prints the whole assembled configuration as one block-style YAML document. Secret references stay references (`{file: …}` or `{env: …}`), and no value is resolved. Compare the counts with the intended inventory. For `examples/config` they are 4, 10 and 13.

The export is a valid single-file configuration: saved as `_index.yaml` in an empty directory, it loads to the same configuration. It does not keep the split file layout or comments, so it is a reference and a disaster-recovery copy, not a replacement for the real tree.

Sanity-check the published image against the rejection cases too:

```sh
for d in /srv/vesper/src/examples/invalid/*/; do
  docker run --rm --network none --user 10001:10001 --read-only -v "$d:/config:ro" \
    "$(sed -n 's/^VESPER_IMAGE=//p' /srv/vesper/.env)" validate >/dev/null 2>&1 \
    && echo "UNEXPECTED ACCEPT $(basename "$d")" || echo "rejected $(basename "$d")"
done
```

**Record:** the verbatim `validate` output and exit code, the export counts, and the result of the rejection loop.

## 8. Networks, reverse proxy and authentication

Use an authentication layer that authorizes only Vesper administrators. A working login alone is insufficient if it admits everyone with an account. Do not deploy a public unauthenticated dashboard, and do not use `VESPER_DEV_MODE` to avoid configuring protection. The following is the Traefik reference path; for another proxy, implement the [provider-neutral contract](security.md#reverse-proxy-and-authentication-contract) and run the same access checks in phase 10.

Create the private ingress network once. It is internal (no internet) and shared only with Traefik:

```sh
docker network create --driver bridge --internal --subnet 172.31.250.0/28 vesper-ingress
docker network connect vesper-ingress traefik     # also add it to Traefik's own compose file
```

Vesper is attached to two networks, and neither publishes a port on the host. `vesper-ingress` is internal: it has no route out and carries only Traefik's requests. The project's own `egress` bridge (named `vesper_egress` by Compose) gives Vesper ordinary outbound NAT access, to LAN check targets within `allowed_cidrs` and to `api.telegram.org:443` for notifications. Allow both in the host firewall if it filters forwarded traffic. Containers outside this project are not attached to `egress`, and the host reaches Vesper there only as an untrusted peer.

Every peer on `vesper-ingress` is trusted as the authenticating proxy (`VESPER_TRUSTED_PROXIES`). Attach nothing else to it except the short-lived admin client in phase 9. The authentication service, if separate, communicates with Traefik on another network and does not need backend access. Management requests from peers on any other network get 403 from Vesper, even with spoofed identity or forwarding headers.

The labels in `compose.yaml` define two routers:

- `vesper`: `Host(status.example.com)` with the required `VESPER_AUTH_MIDDLEWARE`. It covers the UI, static assets, the API and `/healthz`.
- `vesper-heartbeat`: `POST` to exactly `^/api/heartbeat/<id>/<id>/<id>$`, with priority 1000, without proxy authentication, and with a rate limit and a 1 KiB body limit. Vesper still requires the check's bearer token on this route. Do not apply interactive auth at the whole entrypoint where it would also capture heartbeats.

### Option A: existing authentication middleware

If you already use an authentication proxy or identity gateway, reuse its Traefik middleware or middleware chain, with a Vesper-specific administrator policy. Set its exact name and provider in `.env`, for example `VESPER_AUTH_MIDDLEWARE=admin-auth@file`. Verify that it denies unauthenticated and unauthorized requests, including API reads and writes, and fails closed if the auth service is unavailable. Vesper requires no provider cookies, identity headers or OIDC registration of its own.

### Option B: HTTPS BasicAuth at Traefik

For a small admin-only installation without a separate identity service, use Traefik's built-in BasicAuth middleware. It offers passwords, not SSO or MFA. Create a bcrypt htpasswd file with `htpasswd -cB /srv/traefik/secrets/vesper.htpasswd YOUR_ADMIN_USERNAME` (the password is prompted, never a command argument; omit `-c` when adding users later). Restrict the file to Traefik's runtime user and mount it read-only **in Traefik**, at `/run/secrets/vesper-users`. Do not put passwords or hashes in Vesper's YAML, Compose labels or `.env`.

With Traefik's file provider enabled, put this in its dynamic configuration directory:

```yaml
http:
  middlewares:
    vesper-auth:
      basicAuth:
        usersFile: /run/secrets/vesper-users
        removeHeader: true
```

Set `VESPER_AUTH_MIDDLEWARE=vesper-auth@file` in `/srv/vesper/.env`. Use HTTPS only. This middleware applies to the management router, not the heartbeat router: `removeHeader` must not strip heartbeat bearer tokens. Use a unique strong password for each administrator, and see [Traefik's BasicAuth documentation](https://doc.traefik.io/traefik/middlewares/http/basicauth/) for password-file management.

### Option C: Authelia for SSO/MFA (suggested, optional)

For an existing Authelia installation, follow its [Traefik integration guide](https://www.authelia.com/integration/proxies/traefik/) and [session configuration](https://www.authelia.com/configuration/session/introduction/). Configure its HTTPS login portal and session cookie domain to cover `status.example.com`; provision users and MFA in Authelia, not Vesper. This example assumes Traefik can reach `authelia:9091` on a separate private auth network and the standard ForwardAuth endpoint is enabled. It does not provision an Authelia server.

In Traefik's dynamic file configuration:

```yaml
http:
  middlewares:
    authelia:
      forwardAuth:
        address: http://authelia:9091/api/authz/forward-auth
        trustForwardHeader: true
```

Following Authelia's guide, keep Traefik entrypoint `forwardedHeaders.insecure` and `proxyProtocol.insecure` false; trust upstream headers only from your actual upstream proxy addresses, never arbitrary clients. If the auth hop leaves a private trusted network, use verified TLS to Authelia. Identity response headers are unnecessary for Vesper; it does not turn `Remote-User` into an authenticated audit identity.

Merge the Vesper rule from [`deploy/authelia-access-control.yml`](../deploy/authelia-access-control.yml) into the existing `access_control.rules` before any broader matching rule. Change the domain and admin group; keep the `two_factor` policy. Do not overwrite unrelated rules, and add no `bypass` rule for Vesper or its heartbeat prefix. Set `VESPER_AUTH_MIDDLEWARE=authelia@file` in `/srv/vesper/.env` (or `authelia@docker` if you already define the middleware with Docker labels).

**Record:** the selected middleware, where its administrator policy is configured, and `docker network inspect vesper-ingress -f '{{range .Containers}}{{.Name}} {{end}}'`, which must list only Traefik before Vesper starts. `docker compose --project-directory /srv/vesper config --quiet` must succeed after selecting the middleware; this validates interpolation, not authentication. Phase 10 verifies the protection.

## 9. Start and verify populated results

```sh
cd /srv/vesper && docker compose up -d
docker inspect vesper -f 'health={{.State.Health.Status}} user={{.Config.User}} ro={{.HostConfig.ReadonlyRootfs}} capdrop={{.HostConfig.CapDrop}} secopt={{.HostConfig.SecurityOpt}} ports={{.HostConfig.PortBindings}}'
docker compose logs --tail 50 vesper
```

Expected: `health=healthy user=10001:10001 ro=true capdrop=[ALL] secopt=[no-new-privileges:true] ports=map[]`.

For API checks from the host, use a temporary client on the ingress network. It counts as the trusted proxy, so this access is admin-equivalent. That is acceptable only because creating it already requires Docker administrator rights. `--rm` removes the client after each call.

```sh
CURL_IMAGE=curlimages/curl:8.11.1
vesper_api() { # vesper_api METHOD PATH [curl args...]
  docker run --rm --network vesper-ingress "$CURL_IMAGE" -sS -X "$1" "${@:3}" "http://vesper:8080$2"
}
vesper_api GET /api/snapshot | jq '{config_revision, config_error, config_writable, telegram}'
```

Wait at least two check intervals (60 s with the defaults), then list every check:

```sh
vesper_api GET /api/snapshot | jq -r '.machines[].services[] | .id as $s | .checks[] |
  [$s, .id, .kind, .health, (.last_checked // "never"), .message] | @tsv' | column -t -s $'\t'
```

Success criteria:

- `config_error` is `null`.
- Every enabled HTTP or TCP check in an unpaused service has a recent `last_checked`.
- Each `healthy` matches reality, and each failure has an explained `message`, such as a real outage, a wrong port or a destination outside the allowlist.
- Push checks stay `unknown` until their producers run (phase 11).
- Paused services show `paused`.

Then open the public URL in a browser, authenticate through your chosen layer and confirm the dashboard shows the same inventory.

**Record:** the inspect line, the snapshot summary and the full check table, with an explanation for every check that is not `healthy`.

## 10. Proxy, bypass and exposure checks

Run these from another LAN machine, not the Docker host, without authentication cookies, credentials or a client certificate. Protected routes should redirect to your login provider or return 401/403, depending on the auth layer; never Vesper content or a successful API response:

```sh
U=https://status.example.com
code() { curl -sS -o /dev/null -w '%{http_code} %{redirect_url}\n' "$@"; }
code "$U/"                                                     # auth redirect or 401/403, never 200
code "$U/api/snapshot"; code "$U/api/config"; code "$U/api/csrf"; code "$U/healthz"   # protected
code -X POST "$U/api/snapshot"                                 # protected
code -X POST -H 'Content-Type: application/json' -d '{"healthy":true}' \
  "$U/api/heartbeat/docker1/host/containers"                   # 401 from Vesper (no token): bypass reaches Vesper
curl -sS -X POST -d '{"healthy":true}' "$U/api/heartbeat/docker1/host/containers"   # body: {"error": ...}
code "$U/api/heartbeat/docker1/host/containers"                # GET: protected
code -X POST "$U/api/heartbeat/docker1/host"                   # wrong depth: protected
code -X POST "$U/api/heartbeat/docker1/host/containers/extra"  # protected
code -X POST "$U/api/heartbeatx/docker1/host/containers"       # protected
code -X POST "$U/api/heartbeat/a%2Fb/c/d"                      # protected
code "$U/api/snapshot" -H 'Remote-User: admin' -H 'X-Forwarded-For: 127.0.0.1'  # still protected
curl -sS --max-time 5 "http://<docker-host-LAN-IP>:8080/healthz" || echo "not reachable (expected)"
```

On the Docker host, check the untrusted-peer path and the Origin enforcement:

```sh
docker port vesper                                             # prints nothing
docker run --rm --network vesper_egress "$CURL_IMAGE" -sS -o /dev/null -w '%{http_code}\n' http://vesper:8080/api/snapshot   # 403
docker run --rm --network vesper_egress "$CURL_IMAGE" -sS -o /dev/null -w '%{http_code}\n' http://vesper:8080/healthz        # 200 (public liveness)
csrf=$(vesper_api GET /api/csrf | jq -r .token)
vesper_api POST /api/config/reload -o /dev/null -w '%{http_code}\n' -H "X-Vesper-CSRF: $csrf" -H 'Origin: https://evil.example'   # 403
vesper_api POST /api/config/reload -o /dev/null -w '%{http_code}\n' -H "Origin: https://status.example.com"                      # 403 (no token)
vesper_api POST /api/config/reload -w '\n%{http_code}\n' -H "X-Vesper-CSRF: $csrf" -H "Origin: https://status.example.com"      # 2xx
```

The CSRF token is random per process and is not tied to a cookie. Fetch a fresh one after every restart. `/healthz` answers any direct peer with liveness only. Through the external proxy it stays authenticated.

Also test a permitted administrator and, for group-based auth, a signed-in user outside the admin group. Only the administrator should reach the dashboard and APIs. In an isolated test route or an approved maintenance window, make the authentication service unreachable and confirm access is denied, never forwarded unprotected; for BasicAuth, test a wrong password instead. Do not disrupt a shared login service just to run this check.

**Record:** every status code and the admitted/denied-user and auth-failure results. Any successful unauthenticated response on a protected path, or anything other than Vesper's 401 on the token-less heartbeat, is a blocker. An authenticated valid heartbeat must also succeed (phase 11), so a 401 from the proxy's BasicAuth layer is not sufficient evidence. Stop and fix routing before going further.

## 11. Push producers (Docker, Proxmox, jobs)

Producers run on the monitored host. They read local state and `POST` outbound to the heartbeat route. All of them send the token only as `Authorization: Bearer …`, passed to curl on stdin, so it never appears in a URL, the process list or a log. They report `healthy:false` when the host is reachable but unhealthy. When the host or the producer is down, no heartbeat arrives and Vesper opens an incident after `expected_interval_seconds + grace_seconds`.

Common setup on each producer host. Copy the repository's `examples/` directory there first (`git clone` or `scp`), and run the commands from the directory that contains it:

```sh
sudo install -d -m 0755 /usr/local/lib/vesper-push /etc/vesper-push
sudo install -d -m 0700 /etc/vesper-push/tokens
sudo install -m 0755 examples/producers/*.sh examples/watchdog/*.sh /usr/local/lib/vesper-push/
# Copy one check's token from the Vesper host without displaying it:
ssh docker1 'sudo cat /srv/vesper/secrets/push-docker1-host-containers' |
  sudo sh -c 'umask 077; cat > /etc/vesper-push/tokens/docker1-host-containers'
```

If Traefik uses a private CA, install it as `/etc/vesper-push/homelab-ca.pem` and set `VESPER_CA_FILE` in the env file.

**Docker** ([`docker-health.sh`](../examples/producers/docker-health.sh), [unit](../examples/systemd/vesper-docker-health.service)). It reports healthy when `docker info` answers and every container in `VESPER_DOCKER_CONTAINERS` is running and not unhealthy, starting or restarting. Reading Docker state needs the local socket. The `docker` group is root-equivalent, so the unit runs the root-owned script as the dedicated `vesper-push` user with systemd hardening, and receives the token through `LoadCredential`.

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin vesper-push
sudo install -m 0644 examples/systemd/docker-health.env /etc/vesper-push/   # then edit
sudo install -m 0644 examples/systemd/vesper-docker-health.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now vesper-docker-health.timer
sudo systemctl start vesper-docker-health.service; journalctl -u vesper-docker-health.service -n 5 -o cat
```

**Proxmox** ([`proxmox-health.sh`](../examples/producers/proxmox-health.sh), [unit](../examples/systemd/vesper-proxmox-health.service)). It reads `/cluster/resources` and `/cluster/status` with a privilege-separated API token that holds only `PVEAuditor`. It reports healthy when the cluster is quorate (or standalone), the listed nodes are online and the listed VMIDs are running. Its unit uses `DynamicUser=yes`. On the node, as root, with `jq` installed:

```sh
pveum user add vesper@pve --comment 'Vesper push producer (read-only)'
sudo sh -c 'umask 077; pveum user token add vesper@pve push --privsep 1 --output-format json |
  jq -r "\"vesper@pve!push=\" + .value" > /etc/vesper-push/pve-api-token'
pveum acl modify / --tokens 'vesper@pve!push' --roles PVEAuditor
```

Then install `proxmox-health.env` and the unit and timer as for Docker. If `https://localhost:8006` fails certificate verification, set `PVE_API_URL` to the node name that the certificate covers.

**Scheduled jobs** ([`run-and-report.sh`](../examples/producers/run-and-report.sh)). Wrap the job, for example `run-and-report.sh nas/backups/restic -- restic backup …`. It pushes the job's real exit status and keeps that status as its own exit code.

**Prove that each producer reports real health**, not just "alive". Put in a temporary harmless fault and confirm that Vesper shows it, then revert:

- Docker: add a nonexistent container name to `VESPER_DOCKER_CONTAINERS` and run the service once. The journal says `ghost: missing` and the check becomes unhealthy.
- Proxmox: add VMID `999999` to `PVE_GUESTS`.
- Jobs: run `run-and-report.sh <id> -- false`.

**Record:** each producer's journal line, its check's state in the snapshot before, during and after the fault, and confirmation that the fault was reverted.

## 12. Telegram: configure and explicit test

1. The user creates a bot with @BotFather and types the token at a hidden prompt:
   `read -rsp 'Bot token: ' t; printf '%s\n' "$t" | sudo sh -c 'umask 027; cat > /srv/vesper/secrets/telegram-bot-token'; unset t; sudo chown root:10001 /srv/vesper/secrets/telegram-bot-token; sudo chmod 0440 /srv/vesper/secrets/telegram-bot-token`
2. The user messages the bot, or adds it to the target group, then finds the chat ID without printing the token. The Bot API puts the token in the URL by design, so pass it on stdin:
   `sudo sh -c 'printf "url = \"https://api.telegram.org/bot%s/getUpdates\"\n" "$(cat /srv/vesper/secrets/telegram-bot-token)"' | curl -sS --config - | jq '.result[].message.chat | {id, type, title}'`
   Group IDs are negative, for example `-100…`.
3. Put `VESPER_TELEGRAM_CHAT_ID=<id>` in `/srv/vesper/vesper.env`, make sure `telegram.dashboard_url` equals the public origin, then run `docker compose up -d`, which recreates the container to pick up the new environment.
4. Send the explicit test from Settings → Telegram, or with:
   `csrf=$(vesper_api GET /api/csrf | jq -r .token); vesper_api POST /api/telegram/test -H "X-Vesper-CSRF: $csrf" -H 'Origin: https://status.example.com'`
5. **Ask the user** whether the message arrived in the intended chat and whether its dashboard link opens through the chosen authentication layer. A 2xx response only means the Bot API accepted the message.

Typical failures: `401 Unauthorized` means a wrong token. `400 chat not found` means a wrong ID, or the bot was never messaged. `403` means the bot was blocked or removed from the group. `429` means rate limiting; Vesper honours `retry_after`. After a 400, 401, 403 or 404 rejection, Vesper stops delivering until the configuration revision changes or a test succeeds. Fix the cause, then run the test again. The snapshot's `telegram` field shows the current delivery state.

Choose the chat before going live. Vesper has a single recipient. If the chat ID changes, outages already sent to the old chat never get their recovery there.

**Record:** the API response `message`, then exactly one of "delivery confirmed by <person> at <time>" or "accepted by the Bot API, delivery NOT confirmed".

## 13. Failure and recovery drill

Run the drill on a dedicated drill service, never on production checks. Create `machines/docker1/services/drill.yaml` (writable mode: through the editor or as a file; read-only mode: through Git) with its own token file `push-docker1-drill-heartbeat`:

```yaml
name: Vesper drill
description: Temporary failure/recovery drill; remove after installation.
checks:
  heartbeat:
    kind: push
    token: {file: push-docker1-drill-heartbeat}
    expected_interval_seconds: 60
    grace_seconds: 30
  port:
    kind: tcp
    host: 192.168.10.21   # a LAN host in allowed_cidrs with a port known to be open
    port: 22
```

Validate the tree and reload it (phase 7, then `POST /api/config/reload`). Then run these steps, noting the UTC time of each transition from the snapshot or the UI:

1. Send a healthy heartbeat (`vesper-push.sh docker1/drill/heartbeat true` with that token). Confirm the check is healthy and `port` passes.
2. **Push outage:** send nothing more. After about 90 s the service goes Down and an incident opens. The outage notification is sent; ask the user to confirm it arrived.
3. **Push recovery:** send one healthy heartbeat. The incident resolves and the recovery notification is sent.
4. **Explicit unhealthy push:** send `vesper-push.sh docker1/drill/heartbeat false`. The check is Down immediately, without waiting for the deadline. Send `true` to recover it.
5. **Active outage:** change `port` to one that is closed on that host and reload. The check is Suspect at the first failure and Down at the third consecutive failure (60–90 s after the change at 30 s intervals). Exactly one incident opens for the service.
6. **Active recovery:** restore port 22 and reload. The check is Recovering at the first pass and Healthy after 2. The incident resolves.
7. **Silence:** silence the drill service for 10 minutes in the UI and repeat step 2. An incident opens but no notification is sent. Recover it with a heartbeat while the silence is active, then clear the silence. Because the outage was never sent, no recovery is sent either, and the incident stays history-only.
8. Acknowledge one drill incident and confirm it shows as acknowledged. Acknowledging is awareness only; it also stops opt-in reminders.
9. Make sure no drill incident is open, then delete `drill.yaml` (or set `paused: true`), reload, and delete the drill token file. Removing a service that still has an open incident closes it, and sends a CLOSED notification if its outage was delivered. The drill incidents stay in history permanently; label them as a drill in your report.

**Record:** the timeline with timestamps against the expected thresholds, the incident IDs, each notification and whether a person confirmed it, and the cleanup.

## 14. Independent watchdog

Vesper cannot report its own outage. [`vesper-watchdog.sh`](../examples/watchdog/vesper-watchdog.sh) runs every minute on a **different physical machine** from Vesper. Each run, it posts a real heartbeat to Vesper's own push check `docker1/vesper/watchdog`, then pings an Uptime Kuma **Push** monitor: `status=up` when Vesper accepted the heartbeat, `status=down` otherwise. Kuma alerts through its own notification channel when the pings say down or stop arriving. This covers the Vesper process, its database writes, Traefik's heartbeat route, DNS, TLS and the network path. It does not prove that Vesper's scheduler is running active checks or that Telegram delivery works. Kuma's own monitors and the drill cover those.

1. In Kuma, add a monitor of type **Push** with a heartbeat interval of 60 s. Copy its push URL *without* the query string. On the watchdog host, the user writes the URL at a hidden prompt into `/etc/vesper-push/kuma-push-url` (mode 0600). Kuma's push API keeps its token in the URL, so the script hands the URL to curl on stdin and never logs it.
2. Copy the `push-docker1-vesper-watchdog` token to `/etc/vesper-push/tokens/docker1-vesper-watchdog` (phase 11), then install `watchdog.env`, `vesper-watchdog.service` and `vesper-watchdog.timer`, and run `systemctl enable --now vesper-watchdog.timer`.
3. Test it: `docker stop vesper` on the Docker host. Within about 1–2 minutes Kuma shows the monitor down and alerts. Run `docker start vesper` and Kuma recovers. Ask the user to confirm the Kuma alert arrived.

If Kuma runs on the same machine as Vesper, a host failure silences both. Say so in the report and recommend moving one of them.

**Record:** the watchdog host, the journal lines, and the Kuma state during and after the stop test, with the user's confirmation.

## 15. Parallel comparison with Uptime Kuma

Keep Kuma and all its monitors running unchanged. Run both side by side for at least 7 days, or as long as the user chooses. For every incident either system raised, record whether the other saw it, the detection delay, and the reason for any difference: thresholds, vantage point, unsupported check type, or a configuration error. Retiring Kuma monitors is the user's decision. Kuma keeps the watchdog role either way.

## 16. Upgrades

Select a published tag (or digest) from a successful main CI run, pull it, and retain the old digest for rollback. Never rebuild locally for an upgrade. These commands use the `vesper_cli` function from phase 7:

```sh
set -e
requested='ghcr.io/cfbender/vesper:sha-<full-commit-sha>'   # replace with the reviewed published tag
docker pull "$requested"
new=$(docker image inspect --format '{{index .RepoDigests 0}}' "$requested")
old=$(sed -n 's/^VESPER_IMAGE=//p' /srv/vesper/.env)
# Validate the current config with the NEW image before switching:
sudo sed -i "s|^VESPER_IMAGE=.*|VESPER_IMAGE=$new|" /srv/vesper/.env && vesper_cli validate || {
  sudo sed -i "s|^VESPER_IMAGE=.*|VESPER_IMAGE=$old|" /srv/vesper/.env; echo "validation failed; staying on $old"; exit 1; }
```

Stop if pull, inspect or validation fails. Fetch installation assets at the new image's `org.opencontainers.image.revision` as in phase 2, and review changes to the runbook and Compose example without overwriting local proxy settings or secrets. Before running `docker compose up -d` with the new image, take a data backup (phase 17), and keep the old image (do not prune it). Afterwards, repeat the checks in phases 9 and 10 and confirm the watchdog stays up.

**Rollback:** set `VESPER_IMAGE` back to the old digest and run `docker compose up -d`. If the new version migrated the database, an older binary may not open it. In that case, also restore the pre-upgrade backup, and accept that history recorded since the upgrade is lost.

## 17. Backups and restore

| What | How | Notes |
| --- | --- | --- |
| Config | Git, or `tar -C /srv/vesper -czf config-$(date -u +%F).tgz config` | Also keep `vesper_cli export > vesper-export-$(date -u +%F).yaml`. It is a single-file YAML copy with secret references only, and it does not keep the split layout or comments. |
| Data (`vesper.db`) | Stop, copy, start; or an online `sqlite3 .backup` | History, incidents, acknowledgements, silences, audit and delivery state |
| Secrets | A separate encrypted backup, kept apart from the others | Never store secrets with the config backup |
| Deployment | `compose.yaml`, `.env`, `vesper.env` (encrypted), reverse-proxy and authentication configuration | |

Consistent data backup, choose one:

```sh
# a) Brief stop: push checks tolerate the gap within their grace period
cd /srv/vesper && docker compose stop vesper && sudo cp -a data "data-backup-$(date -u +%Y%m%dT%H%M%SZ)" && docker compose start vesper
# b) Online, with sqlite3 installed on the host (local filesystem only)
sudo sqlite3 /srv/vesper/data/vesper.db ".backup '/var/backups/vesper-$(date -u +%Y%m%dT%H%M%SZ).db'"
sudo sqlite3 /var/backups/vesper-<stamp>.db 'PRAGMA integrity_check;'   # "ok"
```

Never copy `vesper.db` alone while the container is running. Without `.backup`, the `-wal` file holds committed data that a plain copy can miss.

**Storage growth.** Samples are capped at 500 per check, so that part of the database is bounded by the number of checks. Incidents, incident events and the audit log are never pruned, so they grow with the number of outages and edits. A flapping check therefore grows the database faster than a stable inventory does. Record `du -sh /srv/vesper/data` after installation and check it monthly, alongside the backup size. Steady growth beyond what the incident count explains means a flapping check or a stuck producer, so investigate that rather than deleting rows. Keep backup retention, for example 7 daily plus 4 weekly, on a different disk from `/srv/vesper`.

Restore:

```sh
cd /srv/vesper && docker compose stop vesper
sudo mv data "data-replaced-$(date -u +%Y%m%dT%H%M%SZ)"
sudo install -d -m 0750 -o 10001 -g 10001 data
sudo install -m 0640 -o 10001 -g 10001 /var/backups/vesper-<stamp>.db data/vesper.db   # no stale -wal/-shm
docker compose start vesper
```

Then confirm that the incident history and recent samples from before the backup are present (`vesper_api GET /api/snapshot`). Do a restore rehearsal once during installation, then restore the live data.

**Record:** the backup method, the location, the result of the integrity check, and the result of the restore rehearsal.

## 18. Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| Container `unhealthy` or restarting | `docker compose logs vesper`: a wrong `VESPER_PUBLIC_URL` (must be an exact `https` origin with no path), `/data` not writable by 10001, or an invalid listen address |
| Dashboard 403 `{error}` through Traefik | Traefik's address is not inside `VESPER_TRUSTED_PROXIES`, or Traefik reached Vesper over a different network (`traefik.docker.network`) |
| Mutations 403 in the browser | The browser origin differs from `VESPER_PUBLIC_URL` (scheme, host or port) |
| Config changes ignored | Read `config_error` in the snapshot: the last valid revision stays active until the files validate |
| UI says export-only | The config is read-only, or the edit cannot be applied safely to the split layout |
| Check says destination not allowed | The resolved address is outside `allowed_cidrs`, or it is loopback or link-local |
| TLS errors for internal services | `private_ca_file` missing, or the certificate does not cover the URL's host |
| Heartbeat 401 | Wrong token or check ID, or the token file has extra content. Vesper deliberately does not say which |
| Heartbeat goes to a login page or gets a proxy auth challenge | The heartbeat router is missing or shadowed, auth is applied to the entire entrypoint, or the request is not POST to exactly three IDs |

## 19. Final report

Report these items:

- The source commit, requested GHCR tag and pinned image digest.
- The configuration mode.
- The inventory counts, and the Kuma mapping with any gaps.
- The validate output.
- The populated check table.
- The status codes from phase 10.
- The producer evidence.
- The Telegram result, in the wording from phase 12.
- The drill timeline.
- The watchdog test.
- The backup and restore evidence.
- Every step not verified, with its reason.
- Open risks, such as Kuma and Vesper sharing a host.
