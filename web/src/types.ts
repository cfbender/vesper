// Mirrors crates/core/src/model.rs and the HTTP contract in docs/architecture.md.

export type Health =
  | "unknown"
  | "healthy"
  | "suspect"
  | "down"
  | "recovering"
  | "stale"
  | "paused";

export type CheckKind = "http" | "tcp" | "push";

/** Serialized as `{env: NAME}` or `{file: filename}`; never a secret value. */
export type SecretRef = { env: string } | { file: string };

export interface CheckDefaults {
  interval_seconds: number;
  timeout_seconds: number;
  failure_threshold: number;
  recovery_threshold: number;
  stale_after_seconds: number;
}

export interface NetworkPolicy {
  allowed_cidrs: string[];
  max_concurrent: number;
  max_response_bytes: number;
  private_ca_file: string | null;
}

export interface Check {
  kind: CheckKind;
  name: string;
  enabled: boolean;
  url: string | null;
  host: string | null;
  port: number | null;
  expected_status: number;
  body_contains: string | null;
  headers: Record<string, SecretRef>;
  follow_redirects: boolean;
  token: SecretRef | null;
  expected_interval_seconds: number;
  grace_seconds: number;
  interval_seconds: number | null;
  timeout_seconds: number | null;
  failure_threshold: number | null;
  recovery_threshold: number | null;
  stale_after_seconds: number | null;
}

export interface MaintenanceWindow {
  start: string;
  end: string;
  repeat_weekly: boolean;
}

export interface Service {
  name: string;
  icon: string;
  description: string;
  paused: boolean;
  notifications: boolean;
  depends_on: string[];
  maintenance: MaintenanceWindow[];
  checks: Record<string, Check>;
}

export interface Machine {
  name: string;
  site: string;
  description: string;
  host_service: string | null;
  services: Record<string, Service>;
}

export interface ProbeResult {
  healthy: boolean;
  latency_ms: number;
  message: string;
}

export interface Sample {
  at: string;
  healthy: boolean;
  latency_ms: number;
  message: string;
}

export interface CheckView {
  id: string;
  name: string;
  kind: CheckKind;
  health: Health;
  last_checked: string | null;
  latency_ms: number | null;
  message: string;
  failures: number;
  successes: number;
  policy: CheckDefaults;
  history: Sample[];
}

export interface ServiceView {
  id: string;
  name: string;
  icon: string;
  description: string;
  health: Health;
  paused: boolean;
  maintenance: boolean;
  silenced_until: string | null;
  source: string;
  checks: CheckView[];
}

export interface MachineView {
  id: string;
  name: string;
  site: string;
  description: string;
  health: Health;
  services: ServiceView[];
}

export interface IncidentEvent {
  at: string;
  message: string;
}

export interface IncidentView {
  id: number;
  service_id: string;
  service_name: string;
  opened_at: string;
  resolved_at: string | null;
  acknowledged_at: string | null;
  notification_state: string;
  correlated_with: string | null;
  events: IncidentEvent[];
}

export interface Snapshot {
  generated_at: string;
  started_at: string;
  config_revision: string;
  config_error: string | null;
  config_writable: boolean;
  machines: MachineView[];
  incidents: IncidentView[];
  telegram: string;
}

export interface ConfigResponse {
  revision: string;
  writable: boolean;
  defaults: CheckDefaults;
  network: NetworkPolicy;
  machines: Record<string, Machine>;
  sources: Record<string, string>;
  telegram_configured: boolean;
}

export interface IconEntry {
  id: string;
  name: string;
  url: string;
}

export interface SaveResponse {
  revision: string;
  message: string;
}
