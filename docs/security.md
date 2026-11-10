# Vesper security model

Vesper is a single-tenant homelab monitor. It has no accounts of its own. Its safety depends on the deployment boundary described here. Treat any deviation from it as a security change.

## Trust boundaries

Browsers connect over HTTPS to a reverse proxy, optionally with authentication. Only the proxy can reach Vesper's management interface over a private connection. Push producers use one narrowly scoped bearer-token route. Vesper connects outbound to allowed LAN destinations and Telegram.

- **Authentication is optional, but strongly recommended.** Vesper has no users, passwords, OIDC or sessions. Without external authentication, anyone who can reach the dashboard or management API has full administrator access: they can edit monitoring YAML (when writable), silence, acknowledge, upload icons and send Telegram tests. Network reachability is then the only access boundary. Limit it to trusted administrators with a firewall or private VPN; unauthenticated public exposure gives the internet those same privileges. Authelia is a suggested SSO/MFA option, with an [optional access-control example](../deploy/authelia-access-control.yml); proxy-native HTTPS BasicAuth is another option. Vesper warns at startup and in Settings but does not detect or require proxy authentication.
- **Trusted peers.** Vesper serves the dashboard and API only to TCP peers inside `VESPER_TRUSTED_PROXIES`. Peers anywhere else get 403. The reference deployment trusts only the internal `vesper-ingress` network, which Traefik alone joins. Anyone who can attach a container to that network can act as an administrator. That is already a Docker administrator, who is root-equivalent on the host. `VESPER_DEV_MODE=true` relaxes this check for local development only. Never set it in a deployment.
- **No published port.** The reference container publishes nothing on the host. Keep the backend reachable only by the reverse proxy, never directly by clients or the internet. Disabling authentication does not disable this peer boundary, HTTPS, Origin/CSRF checks or heartbeat tokens.
- **`/healthz`** answers any direct peer with liveness only: no inventory, no versions, no credentials. At the external proxy it follows the same optional authentication policy as other non-heartbeat routes.

## Reverse-proxy and authentication contract

Traefik is the reference deployment, not a runtime dependency. Another reverse proxy (for example Nginx or Caddy) can meet the same contract; Vesper has no provider-specific authentication integration.

1. Terminate HTTPS at the proxy. If using authentication, protect **every dashboard, static asset and management API route**, including read-only endpoints and `/api/csrf`. Admit only Vesper administrators; authenticating everyone in an organization is not sufficient authorization. Deny access when the authentication service fails. If deliberately running without authentication, restrict network reachability and accept that every reachable client is an administrator; CSRF and trusted-proxy checks do not authenticate users.
2. Route allowed requests to Vesper over a private network. Set `VESPER_TRUSTED_PROXIES` to the **immediate TCP peer addresses** of the proxy, preferably exact `/32` or `/128` addresses, or an isolated subnet containing only trusted proxies and Vesper. Never list all LAN clients or `0.0.0.0/0`. Vesper does not use `Forwarded`, `X-Forwarded-For`, identity headers, provider cookies, or a proxy-supplied username to decide who is an administrator. Requests from a trusted peer are already admin-equivalent; the peer check cannot verify that your proxy actually authenticated them.
3. Set `VESPER_PUBLIC_URL` to the externally visible HTTPS origin without a path. Preserve browser `Origin`, `Sec-Fetch-Site` and `X-Vesper-CSRF`; do not replace an untrusted Origin with the configured one. Preserve the public host/scheme through your auth layer for its redirects. Sanitize client-supplied forwarding and identity headers at your proxy's trust boundary according to its documentation. Vesper does not need identity response headers and does not attribute audit entries to an asserted user.
4. If push producers bypass interactive authentication, exempt only the exact POST heartbeat route below. Preserve its `Authorization: Bearer` header unchanged. Do not exempt the `/api` prefix, all heartbeat methods, or an entire domain. Apply the body and rate limits at the proxy; producers must never receive an HTML login page for a valid heartbeat.
5. Verify that untrusted backend peers get 403 even with spoofed forwarding/identity headers, and management mutations with a missing CSRF token or wrong Origin get 403. With authentication, also verify denied/admitted users and authentication-service failure; a login screen alone does not prove the API is protected. Without authentication, verify your intended network restrictions and that allowed clients get administrator access without logging in.

## Optional authentication examples

Reuse your existing proxy authentication if you have it. With the [Traefik reference Compose file](../deploy/compose.yaml), set `VESPER_AUTH_MIDDLEWARE` to its middleware name and provider, such as `admin-auth@file`. Leaving it empty disables proxy authentication; the administrator-access warning still applies. These examples go in Traefik's dynamic file configuration, not Vesper's YAML.

**BasicAuth:** mount a bcrypt htpasswd file read-only into Traefik at `/run/secrets/vesper-users`, accessible only to its runtime user. Create/manage users with an interactive `htpasswd` prompt, not passwords in command arguments or Compose labels. Use HTTPS; BasicAuth provides passwords, not SSO/MFA. See [Traefik's BasicAuth guide](https://doc.traefik.io/traefik/middlewares/http/basicauth/).

```yaml
http:
  middlewares:
    vesper-auth:
      basicAuth:
        usersFile: /run/secrets/vesper-users
        removeHeader: true
```

Set `VESPER_AUTH_MIDDLEWARE=vesper-auth@file`. Apply it to the management router only: heartbeat requests need their bearer header intact.

**Authelia (SSO/MFA):** for an existing installation, follow the [Traefik integration](https://www.authelia.com/integration/proxies/traefik/) and [session configuration](https://www.authelia.com/configuration/session/introduction/) guides. Its HTTPS login portal and cookie domain must cover Vesper's hostname; provision users and MFA in Authelia. This example assumes Traefik can reach `authelia:9091` on a private auth network:

```yaml
http:
  middlewares:
    authelia:
      forwardAuth:
        address: http://authelia:9091/api/authz/forward-auth
        trustForwardHeader: true
```

Set `VESPER_AUTH_MIDDLEWARE=authelia@file`. Merge the [access-control rule](../deploy/authelia-access-control.yml) before broader matching rules, adapting its domain and administrator group. Don't add an Authelia bypass rule; only the exact heartbeat router omits the auth middleware. Keep Traefik's `forwardedHeaders.insecure` and `proxyProtocol.insecure` false and trust forwarding headers only from actual upstream proxies. Use verified TLS for an auth hop outside a private trusted network. Vesper needs no identity response headers or Authelia backend access.

## The heartbeat exception

When proxy authentication is enabled, exactly one route skips it: `POST` to `/api/heartbeat/<machine>/<service>/<check>`, with three ID segments of `A-Z a-z 0-9 _ -` (1–80 characters each). The Traefik rule uses an anchored `PathRegexp` and `Method(POST)`, at a higher priority than the main router. It adds a per-source rate limit and a 1 KiB body limit. Other methods, depths, prefixes and encoded variants fall through to the management router. Heartbeat tokens are required even when management authentication is disabled. Verify a valid bearer heartbeat succeeds, a token-less one returns Vesper's JSON 401, and other routes still follow your management access policy.

On that route, Vesper:

- requires `Authorization: Bearer <token>`, where the token is the check's own push token. A token is valid for exactly one check;
- reveals nothing on failure. A wrong token, a missing token and an unknown check all give the same 401 `{"error": …}`;
- returns no inventory, needs no CSRF token, and never takes a secret from the URL.

Push producers ([examples/producers](../examples/producers)) send the token only in the header, fed to curl on stdin. The token is never in an argument, a URL or a log. Producers refuse plain-HTTP URLs.

## Browser protections

- Every management mutation requires the `X-Vesper-CSRF` token from `GET /api/csrf`, and an `Origin` header exactly equal to `VESPER_PUBLIC_URL`. That URL must be an `https` origin without a path. The token is random per process and is not a cookie or a session. CSRF protection is not authentication: without external auth, any reachable client can fetch the token and perform mutations with the configured Origin.
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
- Destinations are resolved, validated and then connected to by address, including every redirect hop. Loopback, link-local, cloud metadata, multicast and unspecified addresses are always denied. IPv4-mapped IPv6 addresses are normalized before applying the same rules, so they cannot bypass a denied IPv4 destination.
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

- Anyone admitted by the reverse proxy is a full administrator, whether authenticated or not. Vesper has no user identity of its own. Its audit log records actions without an authenticated username; use your proxy/authentication logs for investigation rather than treating identity headers as Vesper-verified attribution.
- An attacker who holds a check's push token can mark that one check healthy or unhealthy. Rotate the token if it is exposed.
- Monitoring from one vantage point cannot tell a target outage from a path outage. Telegram API acceptance is not delivery proof.
- Vesper cannot detect its own outage. Run the independent watchdog.

## Reporting a vulnerability

Report security problems privately through GitHub's "Report a vulnerability" (security advisories) on [cfbender/vesper](https://github.com/cfbender/vesper/security), not in a public issue.
