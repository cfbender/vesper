// TEST-ONLY FIXTURES. Illustrative inventory for unit tests and the local
// fixture server used to render screenshots. Never imported by src/.
import type {
  NotificationKind,
  Check,
  CheckView,
  ConfigResponse,
  Health,
  IncidentView,
  Machine,
  Sample,
  Service,
  Snapshot,
} from "../src/types.ts";

export function check(patch: Partial<Check>): Check {
  return {
    kind: "http",
    name: "",
    enabled: true,
    url: null,
    host: null,
    port: null,
    expected_status: 200,
    body_contains: null,
    headers: {},
    follow_redirects: false,
    token: null,
    expected_interval_seconds: 300,
    grace_seconds: 60,
    interval_seconds: null,
    timeout_seconds: null,
    failure_threshold: null,
    recovery_threshold: null,
    stale_after_seconds: null,
    ...patch,
  };
}

export function service(patch: Partial<Service>): Service {
  return {
    name: "",
    icon: "server",
    description: "",
    paused: false,
    notifications: true,
    depends_on: [],
    maintenance: [],
    checks: {},
    ...patch,
  };
}

const http = (url: string, patch: Partial<Check> = {}) => check({ kind: "http", name: "Web", url, ...patch });

export function fixtureMachines(): Record<string, Machine> {
  return {
    jelly: {
      name: "Jelly",
      site: "Home",
      description: "Media & automation",
      host_service: "host",
      services: {
        host: service({
          name: "Host",
          description: "Reachability of the machine itself",
          checks: { ssh: check({ kind: "tcp", name: "SSH", host: "192.168.1.20", port: 22 }) },
        }),
        jellyfin: service({
          name: "Jellyfin",
          icon: "jellyfin",
          description: "Media server",
          depends_on: ["jelly/host"],
          checks: {
            web: http("http://192.168.1.20:8096/health", {
              body_contains: "Healthy",
              headers: { "X-Emby-Token": { env: "JELLYFIN_TOKEN" } },
              failure_threshold: 4,
            }),
          },
          maintenance: [{ start: "2026-11-15T03:00:00Z", end: "2026-11-15T04:00:00Z", repeat_weekly: true }],
        }),
        sonarr: service({ name: "Sonarr", icon: "sonarr", depends_on: ["jelly/host"], checks: { web: http("http://192.168.1.20:8989/ping") } }),
        radarr: service({ name: "Radarr", icon: "radarr", depends_on: ["jelly/host"], checks: { web: http("http://192.168.1.20:7878/ping") } }),
        "home-assistant": service({
          name: "Home Assistant",
          icon: "home-assistant",
          checks: { web: http("http://192.168.1.21:8123/") },
        }),
      },
    },
    raptor: {
      name: "Raptor",
      site: "Home",
      description: "Storage & DNS",
      host_service: null,
      services: {
        adguard: service({ name: "AdGuard Home", icon: "adguard-home", checks: { dns: check({ kind: "tcp", name: "DNS", host: "192.168.1.2", port: 53 }), web: http("http://192.168.1.2:3000/") } }),
        backups: service({
          name: "Nightly backups",
          icon: "restic",
          checks: { nightly: check({ kind: "push", name: "Restic run", expected_interval_seconds: 86400, grace_seconds: 7200, token: { file: "backup-heartbeat-token" } }) },
        }),
        grafana: service({ name: "Grafana", icon: "grafana", checks: { web: http("http://192.168.1.2:3001/api/health") } }),
        portainer: service({ name: "Portainer", icon: "portainer", paused: true, checks: { web: http("https://192.168.1.2:9443/") } }),
      },
    },
    bee: {
      name: "Bee",
      site: "Studio",
      description: "Local AI",
      host_service: null,
      services: {
        "open-webui": service({ name: "Open WebUI", icon: "open-webui", checks: { web: http("http://10.0.5.4:8080/health") } }),
        ollama: service({ name: "Ollama", icon: "ollama", checks: { api: http("http://10.0.5.4:11434/") } }),
        halogen: service({ name: "Halogen", description: "In-house inference router", checks: { api: check({ kind: "tcp", host: "10.0.5.4", port: 7100 }) } }),
      },
    },
  };
}

export function fixtureConfig(machines = fixtureMachines(), revision = "r-7c1f3e9a2b44"): ConfigResponse {
  const sources: Record<string, string> = {};
  for (const [m, mc] of Object.entries(machines)) {
    sources[`machines/${m}`] = `machines/${m}/_index.yaml`;
    for (const [s, svc] of Object.entries(mc.services)) {
      sources[`machines/${m}/services/${s}`] = `machines/${m}/services/${s}.yaml`;
      if (m === "jelly" && s === "jellyfin")
        for (const c of Object.keys(svc.checks)) sources[`machines/${m}/services/${s}/checks/${c}`] = `machines/${m}/services/${s}/checks/${c}.yaml`;
    }
  }
  return {
    revision,
    writable: true,
    defaults: { interval_seconds: 30, timeout_seconds: 5, failure_threshold: 3, recovery_threshold: 2, stale_after_seconds: 90 },
    network: { allowed_cidrs: ["192.168.1.0/24", "10.0.5.0/24"], max_concurrent: 16, max_response_bytes: 1048576, private_ca_file: null },
    machines,
    sources,
    notification_channels: [
      { id: "telegram", kind: "telegram", kind_name: "Telegram" },
      { id: "ops", kind: "discord", kind_name: "Discord" },
    ],
  };
}

export type Scenario = "outage" | "healthy" | "empty";

const iso = (ms: number) => new Date(ms).toISOString().replace(/\.\d{3}Z$/, "Z");

function samples(now: number, every: number, failFrom: number | null, n = 30): Sample[] {
  return Array.from({ length: n }, (_, i) => {
    const failed = failFrom !== null && i >= failFrom;
    return {
      at: iso(now - (n - 1 - i) * every * 1000 - 4000),
      healthy: !failed,
      latency_ms: failed ? 5000 : 30 + ((i * 37) % 40),
      message: failed ? "Connection timed out after 5s" : "HTTP 200 in time",
    };
  });
}

export function fixtureSnapshot(
  config: ConfigResponse,
  scenario: Scenario,
  state: { acknowledged: Set<number>; silenced: Record<string, string | null> },
  now = Date.now(),
): Snapshot {
  const down = scenario === "outage" ? new Set(["jelly/host", "jelly/jellyfin", "jelly/sonarr", "jelly/radarr"]) : new Set<string>();
  const suspect = scenario === "outage" ? new Set(["bee/ollama"]) : new Set<string>();
  const machines = scenario === "empty" ? [] : Object.entries(config.machines).map(([m, mc]) => {
    const services = Object.entries(mc.services).map(([s, svc]) => {
      const id = `${m}/${s}`;
      const checks: CheckView[] = Object.entries(svc.checks).map(([c, ck]) => {
        const policy = {
          interval_seconds: ck.interval_seconds ?? config.defaults.interval_seconds,
          timeout_seconds: ck.timeout_seconds ?? config.defaults.timeout_seconds,
          failure_threshold: ck.failure_threshold ?? config.defaults.failure_threshold,
          recovery_threshold: ck.recovery_threshold ?? config.defaults.recovery_threshold,
          stale_after_seconds: ck.stale_after_seconds ?? (ck.kind === "push" ? ck.expected_interval_seconds + ck.grace_seconds : config.defaults.stale_after_seconds),
        };
        const isDown = down.has(id);
        const isSuspect = suspect.has(id);
        let health: Health = svc.paused ? "paused" : isDown ? "down" : isSuspect ? "suspect" : "healthy";
        if (ck.kind === "push") health = "healthy";
        const every = ck.kind === "push" ? 86400 : policy.interval_seconds;
        const history = svc.paused ? [] : samples(now, every, isDown ? 18 : isSuspect ? 29 : null, ck.kind === "push" ? 12 : 30);
        const last = history[history.length - 1];
        return {
          id: `${id}/${c}`,
          name: ck.name || c,
          kind: ck.kind,
          health,
          last_checked: last?.at ?? null,
          latency_ms: ck.kind === "push" ? null : (last?.latency_ms ?? null),
          message: svc.paused ? "" : ck.kind === "push" ? "Heartbeat received: healthy" : isDown ? "Connection timed out after 5s" : ck.kind === "tcp" ? "TCP connect succeeded" : `HTTP ${ck.expected_status}`,
          failures: isDown ? 12 : isSuspect ? 1 : 0,
          successes: isDown || isSuspect ? 0 : 30,
          policy,
          history,
        };
      });
      const health: Health = svc.paused ? "paused" : down.has(id) ? "down" : suspect.has(id) ? "suspect" : "healthy";
      return {
        id,
        name: svc.name,
        icon: svc.icon,
        description: svc.description,
        health,
        paused: svc.paused,
        maintenance: false,
        silenced_until: state.silenced[id] ?? null,
        source: config.sources[`machines/${m}/services/${s}`] ?? "",
        checks,
      };
    });
    const health: Health = services.some((s) => s.health === "down") ? "down" : services.some((s) => s.health === "suspect") ? "suspect" : "healthy";
    return { id: m, name: mc.name, site: mc.site, description: mc.description, health, services };
  });
  const opened = now - 6 * 60_000;
  const incidents: IncidentView[] = [
    {
      id: 41,
      service_id: "raptor/grafana",
      service_name: "Grafana",
      opened_at: iso(now - 3 * 86400_000),
      resolved_at: iso(now - 3 * 86400_000 + 540_000),
      acknowledged_at: null,
      notification_state: "Outage and recovery delivered",
      correlated_with: null,
      events: [
        { at: iso(now - 3 * 86400_000), message: "3 consecutive HTTP failures: HTTP 502" },
        { at: iso(now - 3 * 86400_000 + 540_000), message: "Recovered after 2 consecutive passes" },
      ],
    },
  ];
  if (scenario === "outage") {
    incidents.push(
      {
        id: 42,
        service_id: "jelly/host",
        service_name: "Host",
        opened_at: iso(opened),
        resolved_at: null,
        acknowledged_at: state.acknowledged.has(42) ? iso(now - 60_000) : null,
        notification_state: "Outage delivered to Telegram",
        correlated_with: null,
        events: [
          { at: iso(opened - 60_000), message: "SSH: TCP connect timed out after 5s (1 of 3)" },
          { at: iso(opened), message: "Failure threshold reached; incident opened" },
          { at: iso(opened + 4_000), message: "Telegram accepted the outage notification" },
        ],
      },
      {
        id: 43,
        service_id: "jelly/jellyfin",
        service_name: "Jellyfin",
        opened_at: iso(opened + 30_000),
        resolved_at: null,
        acknowledged_at: state.acknowledged.has(43) ? iso(now - 60_000) : null,
        notification_state: "Grouped under jelly/host",
        correlated_with: "jelly/host",
        events: [
          { at: iso(opened + 30_000), message: "Web: 4 consecutive failures; incident opened" },
          { at: iso(opened + 31_000), message: "Notification grouped with dependency jelly/host" },
        ],
      },
    );
  }
  return {
    generated_at: iso(now),
    started_at: iso(now - 4 * 86400_000),
    config_revision: config.revision,
    config_error: null,
    config_writable: config.writable,
    machines,
    incidents,
    notifications: [
      { id: "telegram", kind: "telegram", kind_name: "Telegram", status: `ok; last message accepted ${iso(now - 3600_000)}` },
      { id: "ops", kind: "discord", kind_name: "Discord", status: "configured" },
    ],
  };
}

export const fixtureKinds: { kinds: NotificationKind[] } = {
  kinds: [
    {
      kind: "telegram",
      name: "Telegram",
      docs: "https://core.telegram.org/bots/api#sendmessage",
      per_incident: false,
      fields: [
        { key: "bot_token", kind: "text", choices: null, help: "Bot token from @BotFather.", required: true, secret: true, default: null, example: null },
        { key: "chat_id", kind: "text", choices: null, help: "Chat, group or channel ID.", required: true, secret: true, default: null, example: null },
        { key: "silent", kind: "bool", choices: null, help: "Deliver without sound.", required: false, secret: false, default: null, example: null },
      ],
    },
    {
      kind: "ntfy",
      name: "ntfy",
      docs: "https://docs.ntfy.sh/publish/#publish-as-json",
      per_incident: false,
      fields: [
        { key: "server_url", kind: "url", choices: null, help: "ntfy server.", required: false, secret: false, default: "https://ntfy.sh", example: null },
        { key: "topic", kind: "text", choices: null, help: "Topic name.", required: true, secret: false, default: null, example: "vesper-alerts" },
        { key: "priority", kind: "choice", choices: ["1", "2", "3", "4", "5"], help: "Priority of problems.", required: false, secret: false, default: "4", example: null },
        { key: "access_token", kind: "text", choices: null, help: "Access token.", required: false, secret: true, default: null, example: null },
      ],
    },
    {
      kind: "pagerduty",
      name: "PagerDuty",
      docs: "https://developer.pagerduty.com/docs/events-api-v2/trigger-events/",
      per_incident: true,
      fields: [
        { key: "integration_key", kind: "text", choices: null, help: "Events API v2 integration key.", required: true, secret: true, default: null, example: null },
      ],
    },
  ],
};
