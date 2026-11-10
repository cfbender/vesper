# Vesper v1 contracts

Vesper is one Rust/Axum process serving a bundled React/TypeScript application. SQLite stores operational history, incidents, acknowledgement, silences, audit events and notification delivery state. YAML is the only configuration authority. There are no built-in accounts or sessions: the external authentication proxy must protect every dashboard and management route. All admitted users are administrators.

## Crates and ownership

`vesper-core` owns models, YAML assembly, validation, source provenance, safe atomic edits and secret resolution. `vesper-probe` executes bounded HTTP and TCP checks with network-policy enforcement. `vesper-engine` owns scheduling, state transitions, persistence and Telegram. `vesper-server` owns HTTP APIs, CSRF/origin protection, uploads and static files. `web` owns the React interface. Public models live in `crates/core/src/model.rs`.

Machine, service and check IDs are path segments restricted to ASCII letters, digits, hyphens and underscores. Full service IDs are `machine/service`; full check IDs are `machine/service/check`. Sorting uses these stable IDs, not current health. Dates in APIs are RFC3339 UTC; the browser displays local time. Secret values never enter API responses or configuration exports.

## Core library interface

Implement `config::load(root: &Path) -> anyhow::Result<LoadedConfig>`, with `LoadedConfig { config: Config, revision: String, sources: BTreeMap<String, String>, writable: bool }`. Sources map logical paths such as `machines/jelly/services/media` to owning root-relative YAML file paths. Directory `_index.yaml` fields and child filename-derived keys cannot collide. Reject duplicate YAML keys, symlinks, path escapes, excessive depth/size, duplicate definitions, file/directory and inline/sibling collisions. Validate before returning. No generic deep merge.

Implement `config::validate(&Config) -> Result<()>`, `config::save_service(root, expected_revision, machine_id, service_id, service: &Service) -> Result<LoadedConfig>` and `config::export_service(&Service) -> Result<String>`. Saving edits the owning file while preserving unrelated values and checks split into child files; if a split layout cannot be safely edited, return an explicit export-only error rather than flattening or losing data. Revision mismatch must reject the edit. New services in directory-backed collections get a new file; support adding a machine via a separate `save_machine(root, expected_revision, machine_id, machine: &Machine)` function. Error text must not expose secret content. Implement `secrets::resolve(&SecretRef, secrets_root: &Path) -> Result<String>` with root confinement and bounded files; references serialize as `{env: NAME}` or `{file: filename}`. Never resolve secrets into exported Config.

## Probe library interface

Implement `pub async fn run(check: &Check, defaults: &CheckDefaults, network: &NetworkPolicy, secrets_root: &Path) -> ProbeResult`. HTTP(S), HTTP expected status/body substring, TCP connect and push heartbeat are v1's useful check inventory. Push cannot be actively run and returns a clear error. Docker/Proxmox health is provided by pushed health with scoped producer credentials, not a Docker socket or shell executor.

The file-controlled `allowed_cidrs` list is deny-by-default. Resolve and validate actual destination addresses, then connect to the validated address (including each redirect); deny cloud metadata and loopback/link-local, IPv4-mapped IPv6 bypasses, multicast and unspecified addresses. No ambient HTTP proxy. HTTP redirects default off and never forward secrets across origins. Keep TLS verification on, support configured private CA. Bound total time, response bytes, concurrency in caller, and sanitize error messages. URL userinfo is forbidden. Secret header values are references.

## Engine library interface

Implement `Engine::open(db_path: &Path, loaded: LoadedConfig, secrets_root: PathBuf) -> Result<Self>`, `snapshot(&self) -> Result<Snapshot>`, `replace_config(&mut self, loaded: LoadedConfig) -> Result<()>`, `set_config_error(&mut self, Option<String>)`, `acknowledge(&mut self, incident_id: i64, acknowledged: bool) -> Result<()>`, `silence(&mut self, service_id: &str, until: Option<DateTime<Utc>>) -> Result<()>`, `heartbeat(&mut self, check_id: &str, healthy: bool) -> Result<()>`, and `recheck(&mut self, service_id: &str) -> Result<()>`. Engine is held in `Arc<tokio::sync::Mutex<Engine>>`; `pub async fn run(engine: Arc<Mutex<Engine>>)` starts the scheduler and notification worker without holding the lock during network IO. `pub async fn test_telegram(engine: Arc<Mutex<Engine>>) -> Result<String>` sends an explicit user-requested test. The server validates heartbeat tokens before calling heartbeat. Server polls configuration root every few seconds, reloads only changed revisions, and preserves last valid configuration on errors. Engine exposes safe `config() -> &Config` for server token lookup and policy-controlled draft checks.

Active defaults: every 30s, 5s timeout, 3 consecutive failures to Down, 2 passes to recover. Suspect begins with first failure; Recovering begins with first pass after Down; failed recovery resets passes and never opens another incident. No overlapping runs. One service incident aggregates checks; all enabled checks must be fresh and healthy to recover. Push: one missed expected cadence plus grace deadline opens an incident, one healthy heartbeat recovers; ticks must not add failed samples. Restart/execution gaps are unknown/stale, never fictional failures. Preserve incidents and delivery state on restart. Pause stops checks and is not healthy. Acknowledge is awareness only. Silences and configured maintenance suppress notifications, not execution. After suppression expires, notify still-open problems and recoveries only for recipients who received the outage; entirely suppressed resolved incidents stay history-only. Explicit `depends_on` service IDs group notifications on confirmed dependency failure; child checks keep running, and get rechecked on parent recovery. Correlation is not proven root cause.

Telegram is outbound-only. Durable retries respect retry_after, expose permission/auth errors, and send current summaries instead of obsolete bursts. Once per incident outage/recovery, reminders opt-in. Safe bounded messages contain names, timestamp/duration and authenticated dashboard link, never probe response contents or secrets. API acceptance is not a read receipt; network ambiguity can duplicate sends.

## HTTP API and UI contract

All JSON errors are `{ "error": "safe explanation" }`. API success responses use JSON. `GET /api/snapshot` returns `Snapshot` above. `GET /api/config` returns `{revision, writable, defaults, machines, sources, telegram_configured}` where machines contain raw configurations with secret references, not values; network policy is read-only and not editable by the service editor. `GET /api/csrf` returns `{token}`. Every management mutation requires `X-Vesper-CSRF` and exact allowed Origin; non-browser CLI clients explicitly set Origin. No permissive CORS. External authentication still protects these endpoints; CSRF is not authentication.

- `POST /api/config/reload` activates valid files, otherwise retains last valid.
- `PUT /api/services/:machine/:service` body `{revision, service: Service}` saves YAML.
- `PUT /api/machines/:machine` body `{revision, machine: Machine}` saves YAML.
- `POST /api/draft/test` body `{checks: {id: Check}}` returns `{results: {id: ProbeResult}}`. Enforces saved file-controlled network policy; never incident processing or notifications.
- `POST /api/services/:machine/:service/recheck` queues active checks.
- `POST /api/services/:machine/:service/silence` body `{until: RFC3339 | null}`.
- `POST /api/incidents/:id/acknowledge` body `{acknowledged: boolean}`.
- `POST /api/telegram/test` returns `{message}`.
- `GET /api/icons` returns `{icons: [{id, name, url}]}`; `POST /api/icons` accepts a multipart `file` upload and returns `{id, url}`. Only bounded raster images or safely rasterized SVG are served; uploaded icons use `upload:<filename>` IDs. Bundled icons use catalog IDs. No runtime CDN.
- `POST /api/heartbeat/:machine/:service/:check` with `Authorization: Bearer <per-check token>` and JSON `{healthy: boolean}`. This exact path alone may bypass interactive proxy auth. No secret in URL, no inventory access, no CSRF for heartbeat. Invalid tokens reveal no check metadata.
- `GET /healthz` is liveness only; contains no inventory or credentials.

UI routes: `/`, `/machines/:machine`, `/services/:machine/:service`, `/services/new`, `/services/:machine/:service/edit`, `/incidents`, `/incidents/:id`, `/settings`. Poll the snapshot without replacing form state. A browser disconnect makes the displayed snapshot stale, not endpoint Down. No simulation fallback in the production UI.

## Visual and editor requirements

Liquid glass claret: base `oklch(16.36% .0318 349.63)`, text `oklch(87.16% .0197 72.55)`, primary `oklch(61.28% .1407 4.1)`, sage success, amber warning, distinct coral failure. Incidents above inventory. Every service visible with local icon, name and dot; machine cards side by side desktop and stacked mobile, always TWO service columns on mobile. Service and machine drilldowns are full pages. History, latest evidence, policy, freshness and source paths must be useful. Touch targets 44px, keyboard focus, reduced motion/transparency, no hover-only functions.

Full-page service editor covers identity, machine (including create), icon catalog/upload, multiple HTTP/TCP/push checks, inherited policies, notifications, draft testing, source path and monitored/paused save. Editing does not change execution until save. Failed draft tests don't block saving. Read-only configuration permits draft/test/export with no hidden database override. Show concurrent-write conflicts and warn about unsaved edits. Never render monitored bodies as HTML.
