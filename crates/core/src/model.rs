use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub defaults: CheckDefaults,
    pub network: NetworkPolicy,
    pub telegram: Option<TelegramConfig>,
    pub machines: BTreeMap<String, Machine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CheckDefaults {
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
    pub stale_after_seconds: u64,
}
impl Default for CheckDefaults {
    fn default() -> Self {
        Self {
            interval_seconds: 30,
            timeout_seconds: 5,
            failure_threshold: 3,
            recovery_threshold: 2,
            stale_after_seconds: 90,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkPolicy {
    pub allowed_cidrs: Vec<String>,
    pub max_concurrent: usize,
    pub max_response_bytes: usize,
    pub private_ca_file: Option<String>,
}
impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            allowed_cidrs: vec![],
            max_concurrent: 16,
            max_response_bytes: 1_048_576,
            private_ca_file: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Machine {
    pub name: String,
    pub site: String,
    pub description: String,
    /// Service ID on this machine whose confirmed failure groups dependents.
    pub host_service: Option<String>,
    pub services: BTreeMap<String, Service>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Service {
    pub name: String,
    pub icon: String,
    pub description: String,
    pub paused: bool,
    pub notifications: bool,
    /// Explicit service keys such as jelly/host. Correlation is not root cause.
    pub depends_on: Vec<String>,
    pub maintenance: Vec<MaintenanceWindow>,
    pub checks: BTreeMap<String, Check>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            name: String::new(),
            icon: "server".into(),
            description: String::new(),
            paused: false,
            notifications: true,
            depends_on: vec![],
            maintenance: vec![],
            checks: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceWindow {
    /// RFC3339 instants. Optional recurrence repeats this duration weekly in UTC.
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub repeat_weekly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Check {
    pub kind: CheckKind,
    pub name: String,
    pub enabled: bool,
    pub url: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub expected_status: u16,
    pub body_contains: Option<String>,
    /// Header values are secret references only, never literal credentials.
    pub headers: BTreeMap<String, SecretRef>,
    pub follow_redirects: bool,
    pub token: Option<SecretRef>,
    pub expected_interval_seconds: u64,
    pub grace_seconds: u64,
    pub interval_seconds: Option<u64>,
    pub timeout_seconds: Option<u64>,
    pub failure_threshold: Option<u32>,
    pub recovery_threshold: Option<u32>,
    pub stale_after_seconds: Option<u64>,
}
impl Default for Check {
    fn default() -> Self {
        Self {
            kind: CheckKind::Http,
            name: String::new(),
            enabled: true,
            url: None,
            host: None,
            port: None,
            expected_status: 200,
            body_contains: None,
            headers: BTreeMap::new(),
            follow_redirects: false,
            token: None,
            expected_interval_seconds: 300,
            grace_seconds: 60,
            interval_seconds: None,
            timeout_seconds: None,
            failure_threshold: None,
            recovery_threshold: None,
            stale_after_seconds: None,
        }
    }
}
impl Check {
    pub fn policy(&self, defaults: &CheckDefaults) -> CheckDefaults {
        CheckDefaults {
            interval_seconds: self.interval_seconds.unwrap_or(defaults.interval_seconds),
            timeout_seconds: self.timeout_seconds.unwrap_or(defaults.timeout_seconds),
            failure_threshold: self.failure_threshold.unwrap_or(defaults.failure_threshold),
            recovery_threshold: self
                .recovery_threshold
                .unwrap_or(defaults.recovery_threshold),
            stale_after_seconds: self
                .stale_after_seconds
                .unwrap_or(defaults.stale_after_seconds),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Http,
    Tcp,
    Push,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SecretRef {
    Env(String),
    File(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TelegramConfig {
    pub bot_token: SecretRef,
    pub chat_id: SecretRef,
    pub dashboard_url: String,
    #[serde(default)]
    pub reminder_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Unknown,
    Healthy,
    Suspect,
    Down,
    Recovering,
    Stale,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub healthy: bool,
    pub latency_ms: u64,
    /// Sanitized, bounded evidence; never response bodies or credential URLs.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckView {
    pub id: String,
    pub name: String,
    pub kind: CheckKind,
    pub health: Health,
    pub last_checked: Option<String>,
    pub latency_ms: Option<u64>,
    pub message: String,
    pub failures: u32,
    pub successes: u32,
    pub policy: CheckDefaults,
    pub history: Vec<Sample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub at: String,
    pub healthy: bool,
    pub latency_ms: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceView {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub health: Health,
    pub paused: bool,
    pub maintenance: bool,
    pub silenced_until: Option<String>,
    pub source: String,
    pub checks: Vec<CheckView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineView {
    pub id: String,
    pub name: String,
    pub site: String,
    pub description: String,
    pub health: Health,
    pub services: Vec<ServiceView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentView {
    pub id: i64,
    pub service_id: String,
    pub service_name: String,
    pub opened_at: String,
    pub resolved_at: Option<String>,
    pub acknowledged_at: Option<String>,
    pub notification_state: String,
    pub correlated_with: Option<String>,
    pub events: Vec<IncidentEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentEvent {
    pub at: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub generated_at: String,
    pub started_at: String,
    pub config_revision: String,
    pub config_error: Option<String>,
    pub config_writable: bool,
    pub machines: Vec<MachineView>,
    pub incidents: Vec<IncidentView>,
    pub telegram: String,
}
