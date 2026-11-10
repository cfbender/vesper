//! Vesper monitoring engine: scheduling, state transitions, SQLite
//! persistence and Telegram delivery.
//!
//! The engine is held in `Arc<tokio::sync::Mutex<Engine>>`. [`run`] drives the
//! scheduler and the notification worker; neither holds the lock during
//! network IO. [`tick`] performs one complete scheduler and delivery pass and
//! is what [`run`] repeats; tests use it with an injected clock, prober and
//! Telegram transport through [`Engine::open_with`].

mod notify;
mod store;
pub mod telegram;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Duration as TimeDelta, Utc};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use store::{fmt_time, CheckRow, State, Store};
use tokio::sync::{Mutex, Notify};
use vesper_core::config::LoadedConfig;
use vesper_core::{
    Check, CheckDefaults, CheckKind, CheckView, Config, Health, IncidentView, Machine, MachineView,
    MaintenanceWindow, NetworkPolicy, ProbeResult, Sample, Service, ServiceView, Snapshot,
};

pub use telegram::{HttpResponse, HttpsTelegram, TelegramRequest, TelegramTransport};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Source of the current time. Production uses [`SystemClock`].
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Executes one active check. Production uses [`NetworkProber`].
pub trait Prober: Send + Sync {
    fn probe<'a>(
        &'a self,
        check_id: &'a str,
        check: &'a Check,
        defaults: &'a CheckDefaults,
        network: &'a NetworkPolicy,
        secrets_root: &'a Path,
    ) -> BoxFuture<'a, ProbeResult>;
}

/// Runs checks with `vesper_probe::run` under the file-controlled network policy.
pub struct NetworkProber;
impl Prober for NetworkProber {
    fn probe<'a>(
        &'a self,
        _check_id: &'a str,
        check: &'a Check,
        defaults: &'a CheckDefaults,
        network: &'a NetworkPolicy,
        secrets_root: &'a Path,
    ) -> BoxFuture<'a, ProbeResult> {
        Box::pin(vesper_probe::run(check, defaults, network, secrets_root))
    }
}

/// Injectable collaborators. [`EngineOptions::production`] is what
/// [`Engine::open`] uses.
pub struct EngineOptions {
    pub clock: Arc<dyn Clock>,
    pub prober: Arc<dyn Prober>,
    pub telegram: Arc<dyn TelegramTransport>,
}

impl EngineOptions {
    pub fn production() -> Result<Self> {
        Ok(Self {
            clock: Arc::new(SystemClock),
            prober: Arc::new(NetworkProber),
            telegram: Arc::new(HttpsTelegram::new()?),
        })
    }
}

/// Undelivered child outages wait this long for an unconfirmed (Suspect)
/// dependency so the notification can be grouped with it.
const DEPENDENCY_GROUPING_WAIT_SECONDS: i64 = 300;
const HISTORY_SAMPLES: usize = 60;
const RESOLVED_INCIDENTS_IN_SNAPSHOT: usize = 100;
const EVENTS_PER_INCIDENT: usize = 100;
const LAST_ALIVE_EVERY_SECONDS: i64 = 15;
const PROBE_TIMEOUT_MARGIN_SECONDS: u64 = 2;

struct Runtime {
    service_id: String,
    kind: CheckKind,
    row: CheckRow,
    /// Serialized check definition; a change resets streaks.
    fingerprint: String,
    /// Enabled and the service is not paused.
    active: bool,
    generation: u64,
    next_due: DateTime<Utc>,
    in_flight: bool,
    recheck: bool,
    /// Start of the current monitoring period (engine start, add, unpause,
    /// definition change). Push deadlines never reach back before it.
    baseline: DateTime<Utc>,
}

pub struct Engine {
    store: Store,
    loaded: LoadedConfig,
    secrets_root: PathBuf,
    config_error: Option<String>,
    started_at: DateTime<Utc>,
    clock: Arc<dyn Clock>,
    prober: Arc<dyn Prober>,
    transport: Arc<dyn TelegramTransport>,
    checks: BTreeMap<String, Runtime>,
    /// Open incident per service ID.
    open: BTreeMap<String, i64>,
    silences: BTreeMap<String, DateTime<Utc>>,
    /// Recipient key of the configured chat, from the last credential check.
    recipient: Option<String>,
    credential_error: Option<String>,
    last_alive_written: DateTime<Utc>,
    wake: Arc<Notify>,
}

struct Job {
    check_id: String,
    generation: u64,
    started: DateTime<Utc>,
    check: Check,
    defaults: CheckDefaults,
    network: NetworkPolicy,
    secrets_root: PathBuf,
    prober: Arc<dyn Prober>,
    timeout: Duration,
}

fn split2(id: &str) -> Option<(&str, &str)> {
    let mut parts = id.split('/');
    let (a, b) = (parts.next()?, parts.next()?);
    parts.next().is_none().then_some((a, b))
}

fn split3(id: &str) -> Option<(&str, &str, &str)> {
    let mut parts = id.split('/');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    parts.next().is_none().then_some((a, b, c))
}

fn find_service<'c>(config: &'c Config, service_id: &str) -> Option<(&'c Machine, &'c Service)> {
    let (m, s) = split2(service_id)?;
    let machine = config.machines.get(m)?;
    Some((machine, machine.services.get(s)?))
}

fn find_check<'c>(config: &'c Config, check_id: &str) -> Option<(&'c Service, &'c Check)> {
    let (m, s, c) = split3(check_id)?;
    let service = config.machines.get(m)?.services.get(s)?;
    Some((service, service.checks.get(c)?))
}

fn display_name(name: &str, id: &str) -> String {
    let name = name.trim();
    let name = if name.is_empty() { id } else { name };
    bounded(name, 80)
}

pub(crate) fn bounded(text: &str, max: usize) -> String {
    let mut out: String = text.chars().filter(|c| !c.is_control()).take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

fn secs(n: u64) -> TimeDelta {
    TimeDelta::seconds(n.min(i64::MAX as u64 / 1000) as i64)
}

/// Whether `now` falls inside the window, repeating weekly in UTC when asked.
pub fn in_maintenance_window(window: &MaintenanceWindow, now: DateTime<Utc>) -> bool {
    let (Some(start), Some(end)) = (
        store::parse_time(&window.start),
        store::parse_time(&window.end),
    ) else {
        return false;
    };
    if end <= start || now < start {
        return false;
    }
    if !window.repeat_weekly {
        return now < end;
    }
    let week = TimeDelta::weeks(1);
    if end - start >= week {
        return true;
    }
    let elapsed = (now - start).num_seconds();
    let k = elapsed / week.num_seconds();
    now < end + week * (k as i32)
}

fn push_deadline(row: &CheckRow, baseline: DateTime<Utc>, check: &Check) -> DateTime<Utc> {
    let base = row.last_heartbeat.map_or(baseline, |t| t.max(baseline));
    base + secs(check.expected_interval_seconds.max(1)) + secs(check.grace_seconds)
}

fn push_fresh(row: &CheckRow, check: &Check, now: DateTime<Utc>) -> bool {
    row.last_heartbeat.is_some_and(|t| {
        now <= t + secs(check.expected_interval_seconds.max(1)) + secs(check.grace_seconds)
    })
}

fn active_fresh(row: &CheckRow, policy: &CheckDefaults, now: DateTime<Utc>) -> bool {
    row.last_checked
        .is_some_and(|t| now - t <= secs(policy.stale_after_seconds))
}

fn reset_streak(row: &mut CheckRow) {
    row.state = match row.state {
        State::Down | State::Recovering => State::Down,
        _ => State::Unknown,
    };
    row.failures = 0;
    row.successes = 0;
}

/// Applies one active sample to the threshold state machine.
fn apply_sample(row: &mut CheckRow, healthy: bool, policy: &CheckDefaults) {
    let fail_at = policy.failure_threshold.max(1);
    let pass_at = policy.recovery_threshold.max(1);
    if healthy {
        row.failures = 0;
        row.successes = row.successes.saturating_add(1);
        row.state = match row.state {
            State::Down | State::Recovering if row.successes < pass_at => State::Recovering,
            _ => State::Healthy,
        };
    } else {
        row.successes = 0;
        row.failures = row.failures.saturating_add(1);
        row.state = match row.state {
            State::Down | State::Recovering => State::Down,
            _ if row.failures >= fail_at => State::Down,
            _ => State::Suspect,
        };
    }
}

fn health_of(state: State) -> Health {
    match state {
        State::Unknown => Health::Unknown,
        State::Healthy => Health::Healthy,
        State::Suspect => Health::Suspect,
        State::Down => Health::Down,
        State::Recovering => Health::Recovering,
    }
}

fn rank(h: Health) -> u8 {
    match h {
        Health::Paused => 0,
        Health::Healthy => 1,
        Health::Unknown => 2,
        Health::Stale => 3,
        Health::Suspect => 4,
        Health::Recovering => 5,
        Health::Down => 6,
    }
}

impl Engine {
    /// Opens (creating if needed) the database and starts from persisted state.
    pub fn open(db_path: &Path, loaded: LoadedConfig, secrets_root: PathBuf) -> Result<Self> {
        Self::open_with(db_path, loaded, secrets_root, EngineOptions::production()?)
    }

    pub fn open_with(
        db_path: &Path,
        loaded: LoadedConfig,
        secrets_root: PathBuf,
        options: EngineOptions,
    ) -> Result<Self> {
        let store = Store::open(db_path)?;
        let now = options.clock.now();
        let open_rows = store.open_incidents()?;
        let mut open = BTreeMap::new();
        for row in &open_rows {
            open.entry(row.service_id.clone()).or_insert(row.id);
        }
        let silences = store.silences()?;
        let stored = store.check_states()?;
        let last_alive = store.meta("last_alive")?;
        let mut engine = Engine {
            store,
            loaded,
            secrets_root,
            config_error: None,
            started_at: now,
            clock: options.clock,
            prober: options.prober,
            transport: options.telegram,
            checks: BTreeMap::new(),
            open,
            silences,
            recipient: None,
            credential_error: None,
            last_alive_written: now,
            wake: Arc::new(Notify::new()),
        };
        engine.sync_checks(now, stored)?;
        let since = last_alive
            .as_deref()
            .and_then(store::parse_time)
            .map_or_else(|| "an unknown time".to_string(), fmt_time);
        for row in &open_rows {
            engine.store.add_event(
                row.id,
                now,
                &format!(
                    "Vesper restarted: no checks ran between {since} and {}. The gap is treated as stale evidence, not as failures.",
                    fmt_time(now)
                ),
            )?;
        }
        engine.store.set_meta("last_alive", &fmt_time(now))?;
        engine.evaluate_all(now)?;
        Ok(engine)
    }

    pub fn config(&self) -> &Config {
        &self.loaded.config
    }

    pub fn loaded(&self) -> &LoadedConfig {
        &self.loaded
    }

    pub fn replace_config(&mut self, loaded: LoadedConfig) -> Result<()> {
        let now = self.clock.now();
        self.loaded = loaded;
        self.sync_checks(now, BTreeMap::new())?;
        self.evaluate_all(now)?;
        self.wake.notify_one();
        Ok(())
    }

    pub fn set_config_error(&mut self, error: Option<String>) {
        self.config_error = error.map(|e| bounded(&e, 1000));
    }

    /// Records a management action. `target` must be an ID, never file contents.
    pub fn audit(&mut self, action: &str, target: &str) -> Result<()> {
        let now = self.clock.now();
        self.store
            .audit(now, &bounded(action, 64), &bounded(target, 256))
    }

    /// Acknowledgement is awareness only: it changes no health, suppression
    /// or recovery notification, but stops opt-in reminders.
    pub fn acknowledge(&mut self, incident_id: i64, acknowledged: bool) -> Result<()> {
        let now = self.clock.now();
        let row = self
            .store
            .incident(incident_id)?
            .ok_or_else(|| anyhow!("incident not found"))?;
        if row.acknowledged_at.is_some() == acknowledged {
            return Ok(());
        }
        self.store
            .set_acknowledged(incident_id, acknowledged.then_some(now))?;
        let (event, action) = if acknowledged {
            ("Acknowledged", "incident.acknowledge")
        } else {
            ("Acknowledgement cleared", "incident.unacknowledge")
        };
        self.store.add_event(incident_id, now, event)?;
        self.store.audit(now, action, &incident_id.to_string())?;
        Ok(())
    }

    /// Temporarily suppresses notifications (not execution) until `until`;
    /// `None` clears the silence.
    pub fn silence(&mut self, service_id: &str, until: Option<DateTime<Utc>>) -> Result<()> {
        let now = self.clock.now();
        if find_service(&self.loaded.config, service_id).is_none() {
            bail!("service not found");
        }
        if until.is_some_and(|u| u <= now) {
            bail!("silence end must be in the future");
        }
        self.store.set_silence(service_id, until)?;
        match until {
            Some(u) => self.silences.insert(service_id.to_string(), u),
            None => self.silences.remove(service_id),
        };
        let event = match until {
            Some(u) => format!("Notifications silenced until {}", fmt_time(u)),
            None => "Silence cleared".to_string(),
        };
        if let Some(&id) = self.open.get(service_id) {
            self.store.add_event(id, now, &event)?;
        }
        self.store.audit(
            now,
            if until.is_some() {
                "service.silence"
            } else {
                "service.unsilence"
            },
            service_id,
        )?;
        self.wake.notify_one();
        Ok(())
    }

    /// Records a pushed heartbeat. The server validates the token first.
    /// Heartbeats for paused services or disabled checks are ignored.
    pub fn heartbeat(&mut self, check_id: &str, healthy: bool) -> Result<()> {
        let now = self.clock.now();
        let (service, check) =
            find_check(&self.loaded.config, check_id).ok_or_else(|| anyhow!("check not found"))?;
        if check.kind != CheckKind::Push {
            bail!("check does not accept heartbeats");
        }
        if service.paused || !check.enabled {
            return Ok(());
        }
        let name = display_name(&check.name, check_id);
        let rt = self
            .checks
            .get_mut(check_id)
            .ok_or_else(|| anyhow!("check not found"))?;
        let prev = rt.row.state;
        rt.row.last_heartbeat = Some(now);
        rt.row.last_checked = Some(now);
        rt.row.latency_ms = None;
        if healthy {
            rt.row.state = State::Healthy;
            rt.row.failures = 0;
            rt.row.successes = rt.row.successes.saturating_add(1);
            rt.row.message = "Healthy heartbeat received".into();
        } else {
            rt.row.state = State::Down;
            rt.row.successes = 0;
            rt.row.failures = rt.row.failures.saturating_add(1);
            rt.row.message = "Producer reported unhealthy".into();
        }
        let sample = Sample {
            at: fmt_time(now),
            healthy,
            latency_ms: 0,
            message: rt.row.message.clone(),
        };
        self.store.record_sample(check_id, &rt.row, &sample)?;
        let service_id = rt.service_id.clone();
        let new = rt.row.state;
        self.transition_event(&service_id, &name, prev, new, now)?;
        self.evaluate_service(&service_id, now)?;
        self.wake.notify_one();
        Ok(())
    }

    /// Queues the service's active checks to run as soon as possible.
    pub fn recheck(&mut self, service_id: &str) -> Result<()> {
        let now = self.clock.now();
        let (_, service) = find_service(&self.loaded.config, service_id)
            .ok_or_else(|| anyhow!("service not found"))?;
        if service.paused {
            bail!("service is paused");
        }
        self.queue_recheck(service_id, now);
        self.store.audit(now, "service.recheck", service_id)?;
        Ok(())
    }

    fn queue_recheck(&mut self, service_id: &str, now: DateTime<Utc>) {
        for rt in self.checks.values_mut() {
            if rt.service_id == service_id && rt.active && rt.kind != CheckKind::Push {
                if rt.in_flight {
                    rt.recheck = true;
                } else {
                    rt.next_due = rt.next_due.min(now);
                }
            }
        }
    }

    /// Reconciles runtime check state with the active configuration.
    fn sync_checks(
        &mut self,
        now: DateTime<Utc>,
        mut stored: BTreeMap<String, CheckRow>,
    ) -> Result<()> {
        let mut seen = BTreeSet::new();
        for (mid, machine) in &self.loaded.config.machines {
            for (sid, service) in &machine.services {
                for (cid, check) in &service.checks {
                    let id = format!("{mid}/{sid}/{cid}");
                    let fingerprint = serde_json::to_string(check)?;
                    let active = check.enabled && !service.paused;
                    match self.checks.get_mut(&id) {
                        Some(rt) => {
                            if rt.fingerprint != fingerprint {
                                rt.fingerprint = fingerprint;
                                rt.kind = check.kind;
                                rt.generation += 1;
                                reset_streak(&mut rt.row);
                                rt.row.message = "Check definition changed".into();
                                rt.next_due = now;
                                rt.baseline = now;
                                self.store.save_check(&id, &rt.row)?;
                            }
                            if active && !rt.active {
                                rt.next_due = now;
                                rt.baseline = now;
                            }
                            rt.active = active;
                        }
                        None => {
                            let row = stored.remove(&id).unwrap_or_default();
                            self.checks.insert(
                                id.clone(),
                                Runtime {
                                    service_id: format!("{mid}/{sid}"),
                                    kind: check.kind,
                                    row,
                                    fingerprint,
                                    active,
                                    generation: 0,
                                    next_due: now,
                                    in_flight: false,
                                    recheck: false,
                                    baseline: now,
                                },
                            );
                        }
                    }
                    seen.insert(id);
                }
            }
        }
        let removed: Vec<String> = self
            .checks
            .keys()
            .filter(|id| !seen.contains(*id))
            .cloned()
            .collect();
        for id in removed.iter().chain(stored.keys()) {
            self.checks.remove(id);
            self.store.delete_check(id)?;
        }
        Ok(())
    }

    fn evaluate_all(&mut self, now: DateTime<Utc>) -> Result<()> {
        let removed: Vec<(String, i64)> = self
            .open
            .iter()
            .filter(|(sid, _)| find_service(&self.loaded.config, sid).is_none())
            .map(|(s, i)| (s.clone(), *i))
            .collect();
        for (sid, id) in removed {
            self.store.resolve_incident(
                id,
                now,
                "removed",
                "Closed: service removed from configuration",
            )?;
            self.open.remove(&sid);
        }
        let services: Vec<String> = self
            .loaded
            .config
            .machines
            .iter()
            .flat_map(|(m, machine)| machine.services.keys().map(move |s| format!("{m}/{s}")))
            .collect();
        for sid in services {
            self.evaluate_service(&sid, now)?;
        }
        Ok(())
    }

    /// Explicit dependencies: `depends_on` plus the machine's `host_service`.
    fn parents(&self, service_id: &str) -> Vec<String> {
        let Some((machine, service)) = find_service(&self.loaded.config, service_id) else {
            return vec![];
        };
        let mut parents = service.depends_on.clone();
        if let Some(host) = &machine.host_service {
            let mid = service_id.split('/').next().unwrap_or_default();
            parents.push(if host.contains('/') {
                host.clone()
            } else {
                format!("{mid}/{host}")
            });
        }
        parents.retain(|p| p != service_id);
        parents.sort();
        parents.dedup();
        parents
    }

    fn children(&self, service_id: &str) -> Vec<String> {
        let mut out = vec![];
        for (mid, machine) in &self.loaded.config.machines {
            for sid in machine.services.keys() {
                let child = format!("{mid}/{sid}");
                if self.parents(&child).iter().any(|p| p == service_id) {
                    out.push(child);
                }
            }
        }
        out
    }

    fn check_fresh_healthy(&self, check_id: &str, rt: &Runtime, now: DateTime<Utc>) -> bool {
        let Some((_, check)) = find_check(&self.loaded.config, check_id) else {
            return false;
        };
        rt.row.state == State::Healthy
            && match rt.kind {
                CheckKind::Push => push_fresh(&rt.row, check, now),
                _ => active_fresh(&rt.row, &check.policy(&self.loaded.config.defaults), now),
            }
    }

    fn check_health(&self, check_id: &str, rt: &Runtime, now: DateTime<Utc>) -> Health {
        let Some((service, check)) = find_check(&self.loaded.config, check_id) else {
            return Health::Unknown;
        };
        if service.paused || !check.enabled {
            return Health::Paused;
        }
        match rt.kind {
            CheckKind::Push => {
                if rt.row.state == State::Down {
                    Health::Down
                } else if rt.row.last_heartbeat.is_none() {
                    Health::Unknown
                } else if !push_fresh(&rt.row, check, now) {
                    Health::Stale
                } else {
                    health_of(rt.row.state)
                }
            }
            _ => {
                let policy = check.policy(&self.loaded.config.defaults);
                if rt.row.last_checked.is_none() {
                    Health::Unknown
                } else if !active_fresh(&rt.row, &policy, now) {
                    Health::Stale
                } else {
                    health_of(rt.row.state)
                }
            }
        }
    }

    fn service_checks(&self, service_id: &str) -> impl Iterator<Item = (&String, &Runtime)> {
        let prefix = format!("{service_id}/");
        self.checks
            .range(prefix.clone()..)
            .take_while(move |(id, _)| id.starts_with(&prefix))
    }

    fn service_health(&self, service_id: &str, now: DateTime<Utc>) -> Health {
        let Some((_, service)) = find_service(&self.loaded.config, service_id) else {
            return Health::Unknown;
        };
        if service.paused {
            return Health::Paused;
        }
        let enabled: Vec<(&String, &Runtime)> = self
            .service_checks(service_id)
            .filter(|(id, _)| find_check(&self.loaded.config, id).is_some_and(|(_, c)| c.enabled))
            .collect();
        if self.open.contains_key(service_id) {
            return if enabled.iter().any(|(_, rt)| rt.row.state == State::Down) {
                Health::Down
            } else {
                Health::Recovering
            };
        }
        enabled
            .iter()
            .map(|(id, rt)| self.check_health(id, rt, now))
            .max_by_key(|h| rank(*h))
            .unwrap_or(Health::Unknown)
    }

    fn in_maintenance(&self, service_id: &str, now: DateTime<Utc>) -> bool {
        find_service(&self.loaded.config, service_id)
            .is_some_and(|(_, s)| s.maintenance.iter().any(|w| in_maintenance_window(w, now)))
    }

    fn active_silence(&self, service_id: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.silences.get(service_id).copied().filter(|u| *u > now)
    }

    /// Why notifications for the service are currently held back, if they are.
    fn suppression(&self, service_id: &str, now: DateTime<Utc>) -> Option<String> {
        let (_, service) = find_service(&self.loaded.config, service_id)?;
        if service.paused {
            return Some("service is paused".into());
        }
        if let Some(until) = self.active_silence(service_id, now) {
            return Some(format!("silenced until {}", fmt_time(until)));
        }
        if self.in_maintenance(service_id, now) {
            return Some("maintenance window".into());
        }
        None
    }

    fn transition_event(
        &self,
        service_id: &str,
        check_name: &str,
        prev: State,
        new: State,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let Some(&id) = self.open.get(service_id) else {
            return Ok(());
        };
        let message = match (prev, new) {
            (State::Down, State::Recovering) => format!("{check_name} passed; recovering"),
            (State::Recovering, State::Down) => {
                format!("{check_name} failed during recovery; recovery progress reset")
            }
            (State::Down | State::Recovering, State::Healthy) => {
                format!("{check_name} recovered")
            }
            (p, State::Down) if p != State::Down => format!("{check_name} confirmed down"),
            _ => return Ok(()),
        };
        self.store.add_event(id, now, &message)
    }

    /// Opens or resolves the service's single aggregate incident.
    fn evaluate_service(&mut self, service_id: &str, now: DateTime<Utc>) -> Result<()> {
        let Some((_, service)) = find_service(&self.loaded.config, service_id) else {
            return Ok(());
        };
        if service.paused {
            return Ok(());
        }
        let service_name = display_name(&service.name, service_id);
        let mut down = vec![];
        let mut enabled = 0;
        let mut all_fresh_healthy = true;
        for (id, rt) in self.service_checks(service_id) {
            let Some((_, check)) = find_check(&self.loaded.config, id) else {
                continue;
            };
            if !check.enabled {
                continue;
            }
            enabled += 1;
            if rt.row.state == State::Down {
                down.push(display_name(&check.name, id));
            }
            all_fresh_healthy &= self.check_fresh_healthy(id, rt, now);
        }
        match self.open.get(service_id).copied() {
            None if !down.is_empty() => {
                let correlated = self
                    .parents(service_id)
                    .into_iter()
                    .find(|p| self.open.contains_key(p));
                let mut event = format!("Confirmed down: {}", down.join(", "));
                if let Some(p) = &correlated {
                    event.push_str(&format!(
                        ". Dependency {p} is also down (correlation, not proven root cause)"
                    ));
                }
                let id = self.store.open_incident(
                    service_id,
                    &service_name,
                    now,
                    correlated.as_deref(),
                    &event,
                )?;
                self.open.insert(service_id.to_string(), id);
                for child in self.children(service_id) {
                    let Some(&child_incident) = self.open.get(&child) else {
                        continue;
                    };
                    if let Some(row) = self.store.incident(child_incident)? {
                        if row.correlated_with.is_none() {
                            self.store.set_correlation(child_incident, service_id)?;
                            self.store.add_event(
                                child_incident,
                                now,
                                &format!("Dependency {service_id} confirmed down (correlation, not proven root cause)"),
                            )?;
                        }
                    }
                }
            }
            Some(id) if enabled == 0 => {
                self.store.resolve_incident(
                    id,
                    now,
                    "no_checks",
                    "Closed: service has no enabled checks",
                )?;
                self.open.remove(service_id);
            }
            Some(id) if all_fresh_healthy => {
                self.store.resolve_incident(
                    id,
                    now,
                    "recovered",
                    &format!("Recovered: all {enabled} enabled checks are fresh and healthy"),
                )?;
                self.open.remove(service_id);
                for child in self.children(service_id) {
                    if let Some(&child_incident) = self.open.get(&child) {
                        self.store.add_event(
                            child_incident,
                            now,
                            &format!("Dependency {service_id} recovered; rechecking"),
                        )?;
                    }
                    self.queue_recheck(&child, now);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Push deadlines and periodic bookkeeping. Never adds samples.
    fn housekeeping(&mut self, now: DateTime<Utc>) -> Result<()> {
        let mut missed = vec![];
        for (id, rt) in self.checks.iter_mut() {
            if rt.kind != CheckKind::Push || !rt.active || rt.row.state == State::Down {
                continue;
            }
            let Some((_, check)) = find_check(&self.loaded.config, id) else {
                continue;
            };
            let deadline = push_deadline(&rt.row, rt.baseline, check);
            if now > deadline {
                let prev = rt.row.state;
                rt.row.state = State::Down;
                rt.row.successes = 0;
                rt.row.failures = rt.row.failures.saturating_add(1);
                rt.row.message = format!("No heartbeat by deadline {}", fmt_time(deadline));
                self.store.save_check(id, &rt.row)?;
                missed.push((
                    id.clone(),
                    rt.service_id.clone(),
                    display_name(&check.name, id),
                    prev,
                ));
            }
        }
        for (_, service_id, name, prev) in &missed {
            self.transition_event(service_id, name, *prev, State::Down, now)?;
            self.evaluate_service(service_id, now)?;
        }
        if !missed.is_empty() {
            self.wake.notify_one();
        }
        if now - self.last_alive_written >= TimeDelta::seconds(LAST_ALIVE_EVERY_SECONDS) {
            self.store.set_meta("last_alive", &fmt_time(now))?;
            self.last_alive_written = now;
        }
        Ok(())
    }

    /// Claims due active checks, bounded by `network.max_concurrent` in flight.
    fn take_due(&mut self, now: DateTime<Utc>) -> Vec<Job> {
        let max = self.loaded.config.network.max_concurrent.max(1);
        let mut running = self.checks.values().filter(|rt| rt.in_flight).count();
        let mut due: Vec<(DateTime<Utc>, String)> = self
            .checks
            .iter()
            .filter(|(_, rt)| {
                rt.active && rt.kind != CheckKind::Push && !rt.in_flight && rt.next_due <= now
            })
            .map(|(id, rt)| (rt.next_due, id.clone()))
            .collect();
        due.sort();
        let mut jobs = vec![];
        for (_, id) in due {
            if running >= max {
                break;
            }
            let Some((_, check)) = find_check(&self.loaded.config, &id) else {
                continue;
            };
            let policy = check.policy(&self.loaded.config.defaults);
            let rt = self.checks.get_mut(&id).expect("due check exists");
            rt.in_flight = true;
            rt.recheck = false;
            running += 1;
            jobs.push(Job {
                check_id: id,
                generation: rt.generation,
                started: now,
                check: check.clone(),
                defaults: self.loaded.config.defaults.clone(),
                network: self.loaded.config.network.clone(),
                secrets_root: self.secrets_root.clone(),
                prober: self.prober.clone(),
                timeout: Duration::from_secs(
                    policy.timeout_seconds.max(1) + PROBE_TIMEOUT_MARGIN_SECONDS,
                ),
            });
        }
        jobs
    }

    fn complete(&mut self, job: &Job, result: ProbeResult) -> Result<()> {
        let now = self.clock.now();
        let Some(rt) = self.checks.get_mut(&job.check_id) else {
            return Ok(());
        };
        rt.in_flight = false;
        if rt.generation != job.generation || !rt.active {
            return Ok(()); // definition changed or paused meanwhile: discard
        }
        let Some((_, check)) = find_check(&self.loaded.config, &job.check_id) else {
            return Ok(());
        };
        let policy = check.policy(&self.loaded.config.defaults);
        let name = display_name(&check.name, &job.check_id);
        let prev = rt.row.state;
        if rt.row.last_checked.is_some() && !active_fresh(&rt.row, &policy, now) {
            // Execution gap: never stitch streaks across stale evidence.
            reset_streak(&mut rt.row);
        }
        apply_sample(&mut rt.row, result.healthy, &policy);
        let message = bounded(&result.message, 300);
        rt.row.last_checked = Some(now);
        rt.row.latency_ms = Some(result.latency_ms);
        rt.row.message = message.clone();
        rt.next_due = if rt.recheck {
            rt.recheck = false;
            now
        } else {
            (job.started + secs(policy.interval_seconds.max(1))).max(now)
        };
        let sample = Sample {
            at: fmt_time(now),
            healthy: result.healthy,
            latency_ms: result.latency_ms,
            message,
        };
        self.store.record_sample(&job.check_id, &rt.row, &sample)?;
        let new = rt.row.state;
        let service_id = rt.service_id.clone();
        self.transition_event(&service_id, &name, prev, new, now)?;
        self.evaluate_service(&service_id, now)?;
        if prev != new {
            self.wake.notify_one();
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let now = self.clock.now();
        let config = &self.loaded.config;
        let mut machines = vec![];
        for (mid, machine) in &config.machines {
            let mut services = vec![];
            for (sid, service) in &machine.services {
                let service_id = format!("{mid}/{sid}");
                let mut checks = vec![];
                for (cid, check) in &service.checks {
                    let id = format!("{service_id}/{cid}");
                    let Some(rt) = self.checks.get(&id) else {
                        continue;
                    };
                    checks.push(CheckView {
                        name: display_name(&check.name, cid),
                        kind: check.kind,
                        health: self.check_health(&id, rt, now),
                        last_checked: rt.row.last_checked.map(fmt_time),
                        latency_ms: rt.row.latency_ms,
                        message: rt.row.message.clone(),
                        failures: rt.row.failures,
                        successes: rt.row.successes,
                        policy: check.policy(&config.defaults),
                        history: self.store.samples(&id, HISTORY_SAMPLES)?,
                        id,
                    });
                }
                let source = [
                    format!("machines/{mid}/services/{sid}"),
                    format!("machines/{mid}"),
                    "machines".to_string(),
                ]
                .iter()
                .find_map(|k| self.loaded.sources.get(k).cloned())
                .unwrap_or_default();
                services.push(ServiceView {
                    name: display_name(&service.name, sid),
                    icon: service.icon.clone(),
                    description: service.description.clone(),
                    health: self.service_health(&service_id, now),
                    paused: service.paused,
                    maintenance: self.in_maintenance(&service_id, now),
                    silenced_until: self.active_silence(&service_id, now).map(fmt_time),
                    source,
                    checks,
                    id: service_id,
                });
            }
            let active: Vec<Health> = services
                .iter()
                .map(|s| s.health)
                .filter(|h| *h != Health::Paused)
                .collect();
            let health = if services.is_empty() {
                Health::Unknown
            } else if active.is_empty() {
                Health::Paused
            } else {
                active.into_iter().max_by_key(|h| rank(*h)).unwrap()
            };
            machines.push(MachineView {
                id: mid.clone(),
                name: display_name(&machine.name, mid),
                site: machine.site.clone(),
                description: machine.description.clone(),
                health,
                services,
            });
        }
        let mut incidents = vec![];
        for row in self
            .store
            .recent_incidents(RESOLVED_INCIDENTS_IN_SNAPSHOT)?
        {
            incidents.push(IncidentView {
                id: row.id,
                service_id: row.service_id.clone(),
                service_name: row.service_name.clone(),
                opened_at: fmt_time(row.opened_at),
                resolved_at: row.resolved_at.map(fmt_time),
                acknowledged_at: row.acknowledged_at.map(fmt_time),
                notification_state: self.notification_state(&row, now)?,
                correlated_with: row.correlated_with.clone(),
                events: self.store.events(row.id, EVENTS_PER_INCIDENT)?,
            });
        }
        Ok(Snapshot {
            generated_at: fmt_time(now),
            started_at: fmt_time(self.started_at),
            config_revision: self.loaded.revision.clone(),
            config_error: self.config_error.clone(),
            config_writable: self.loaded.writable,
            machines,
            incidents,
            telegram: self.telegram_status(now)?,
        })
    }
}

async fn execute(engine: Arc<Mutex<Engine>>, job: Job) {
    let started = std::time::Instant::now();
    let result = match tokio::time::timeout(
        job.timeout,
        job.prober.probe(
            &job.check_id,
            &job.check,
            &job.defaults,
            &job.network,
            &job.secrets_root,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => ProbeResult {
            healthy: false,
            latency_ms: started.elapsed().as_millis() as u64,
            message: "check exceeded its time limit".into(),
        },
    };
    let mut engine = engine.lock().await;
    if let Err(e) = engine.complete(&job, result) {
        tracing::error!(check = %job.check_id, "failed to record check result: {e:#}");
    }
}

fn claim(engine: &mut Engine) -> Vec<Job> {
    let now = engine.clock.now();
    if let Err(e) = engine.housekeeping(now) {
        tracing::error!("engine housekeeping failed: {e:#}");
    }
    engine.take_due(now)
}

/// One full pass: push deadlines, run every due check to completion (bounded
/// and without overlap), then one Telegram delivery attempt.
pub async fn tick(engine: &Arc<Mutex<Engine>>) {
    let jobs = claim(&mut *engine.lock().await);
    let mut set = tokio::task::JoinSet::new();
    for job in jobs {
        set.spawn(execute(engine.clone(), job));
    }
    while set.join_next().await.is_some() {}
    {
        let mut e = engine.lock().await;
        let now = e.clock.now();
        if let Err(err) = e.housekeeping(now) {
            tracing::error!("engine housekeeping failed: {err:#}");
        }
    }
    notify::deliver_once(engine).await;
}

/// Starts the scheduler and the notification worker. Never returns.
pub async fn run(engine: Arc<Mutex<Engine>>) {
    let wake = engine.lock().await.wake.clone();
    let worker = engine.clone();
    tokio::spawn(async move {
        loop {
            let attempted = notify::deliver_once(&worker).await;
            // At most one message per second keeps within Telegram chat limits.
            let pause = tokio::time::sleep(Duration::from_secs(1));
            if attempted {
                pause.await;
            } else {
                tokio::select! {
                    _ = wake.notified() => {}
                    _ = pause => {}
                }
            }
        }
    });
    loop {
        let jobs = claim(&mut *engine.lock().await);
        for job in jobs {
            tokio::spawn(execute(engine.clone(), job));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Sends an explicit, user-requested Telegram test message.
pub async fn test_telegram(engine: Arc<Mutex<Engine>>) -> Result<String> {
    notify::test_telegram(&engine).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> CheckDefaults {
        CheckDefaults::default()
    }

    #[test]
    fn thresholds_follow_suspect_down_recovering() {
        let mut row = CheckRow::default();
        let p = policy();
        apply_sample(&mut row, false, &p);
        assert_eq!(row.state, State::Suspect);
        apply_sample(&mut row, false, &p);
        apply_sample(&mut row, false, &p);
        assert_eq!(row.state, State::Down);
        apply_sample(&mut row, true, &p);
        assert_eq!(row.state, State::Recovering);
        apply_sample(&mut row, false, &p);
        assert_eq!((row.state, row.successes), (State::Down, 0));
        apply_sample(&mut row, true, &p);
        apply_sample(&mut row, true, &p);
        assert_eq!(row.state, State::Healthy);
    }

    #[test]
    fn weekly_window_repeats_in_utc() {
        let w = MaintenanceWindow {
            start: "2026-11-01T02:00:00Z".into(),
            end: "2026-11-01T04:00:00Z".into(),
            repeat_weekly: true,
        };
        let at = |s: &str| store::parse_time(s).unwrap();
        assert!(!in_maintenance_window(&w, at("2026-10-25T03:00:00Z")));
        assert!(in_maintenance_window(&w, at("2026-11-01T03:00:00Z")));
        assert!(in_maintenance_window(&w, at("2026-11-15T03:59:59Z")));
        assert!(!in_maintenance_window(&w, at("2026-11-15T04:00:00Z")));
        assert!(!in_maintenance_window(&w, at("2026-11-16T03:00:00Z")));
        let once = MaintenanceWindow {
            repeat_weekly: false,
            ..w
        };
        assert!(!in_maintenance_window(&once, at("2026-11-08T03:00:00Z")));
    }
}
