//! Notification decisions and delivery. Delivery is pull-based: every attempt
//! is composed from the current incident state and the persisted per-recipient
//! delivery marks, so a delayed or rate-limited channel sends one current
//! summary instead of a burst of obsolete events. Each configured channel is
//! an independent recipient with its own marks, rate limits and backoff.

use crate::store::{fmt_time, ChannelRow, IncidentRow};
use crate::transport::{scrub, Network, NotifyTransport};
use crate::DEPENDENCY_GROUPING_WAIT_SECONDS;
use crate::{bounded, display_name, find_check, find_service, Engine};
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Duration as TimeDelta, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use vesper_core::{secrets, ChannelView, Health, SecretRef};
use vesper_notify::{IncidentRef, Message, Provider, Request, Setting, Settings, Status, Verdict};

const MAX_ITEMS_PER_MESSAGE: usize = 25;
const MAX_CHECK_NAMES: usize = 3;
/// Room for the "Vesper: …" header line.
const HEADER_RESERVE: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Outage,
    Reminder,
    Recovery,
}

/// What the last resolution of a channel's settings found.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChannelState {
    /// Recipient key of the channel's destination.
    pub recipient: Option<String>,
    pub error: Option<String>,
}

/// A channel ready to send, with its settings resolved.
pub(crate) struct Resolved {
    id: String,
    provider: &'static Provider,
    settings: Settings,
    recipient: String,
    /// Resolved credential values, removed from every error text.
    secrets: Vec<String>,
    dashboard: String,
}

pub(crate) struct Delivery {
    recipient: String,
    label: String,
    revision: String,
    plan: Result<Vec<Request>, String>,
    secrets: Vec<String>,
    network: Network,
    outage: Vec<i64>,
    recovery: Vec<i64>,
    reminder: Vec<i64>,
}

/// `<channel>:<hash of the destination>`. Delivery marks and channel state
/// belong to this key, so a new destination starts with nothing delivered.
fn recipient_key(channel: &str, provider: &Provider, settings: &Settings) -> String {
    let keys: Vec<&str> = if provider.identity.is_empty() {
        provider.fields.iter().map(|f| f.key).collect()
    } else {
        provider.identity.to_vec()
    };
    let identity: Vec<String> = keys
        .iter()
        .filter_map(|k| match settings.values().get(*k)? {
            Setting::Text(t) => Some(t.trim().to_string()),
            Setting::List(items) => Some(items.join(",")),
        })
        .collect();
    let digest = Sha256::digest(identity.join("\n").as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{channel}:{hex}")
}

/// `Telegram` for a channel named after its kind, else `Telegram (family)`.
fn label(channel: &str, provider: &Provider) -> String {
    if channel == provider.kind {
        provider.name.to_string()
    } else {
        format!("{} ({})", provider.name, bounded(channel, 80))
    }
}

fn fmt_display(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn fmt_duration(d: TimeDelta) -> String {
    let s = d.num_seconds().max(0);
    let (days, hours, minutes) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{s}s")
    }
}

fn backoff(failures: u32) -> TimeDelta {
    let exp = failures.saturating_sub(1).min(7);
    TimeDelta::seconds((5i64 << exp).min(600))
}

/// Sends a plan's requests in order; every one must be accepted. A token
/// step's token joins `secrets` so it is scrubbed from error texts too.
async fn execute(
    transport: &dyn NotifyTransport,
    plan: &[Request],
    network: &Network,
    secrets: &mut Vec<String>,
) -> Verdict {
    for request in plan {
        let verdict = match request {
            Request::Mail(mail) => transport.mail(mail, network).await,
            Request::Http(http) => {
                let mut http = http.clone();
                if let Some(step) = http.token.take() {
                    let response = match transport.http(&step.request, network).await {
                        Ok(r) => r,
                        Err(v) => return v,
                    };
                    match (step.request.accept)(&response) {
                        Verdict::Accepted => {}
                        other => return other,
                    }
                    let Some(token) = response
                        .json()
                        .pointer(step.pointer)
                        .and_then(Value::as_str)
                        .map(str::to_string)
                    else {
                        return Verdict::Rejected("token response has no token".into());
                    };
                    secrets.push(token.clone());
                    http.headers
                        .push((step.header.to_string(), format!("{}{token}", step.prefix)));
                }
                match transport.http(&http, network).await {
                    Ok(response) => (http.accept)(&response),
                    Err(v) => v,
                }
            }
        };
        if verdict != Verdict::Accepted {
            return verdict;
        }
    }
    Verdict::Accepted
}

/// Prefixes the provider name and removes credentials from verdict texts.
fn described(verdict: Verdict, name: &str, secrets: &[String]) -> Verdict {
    match verdict {
        Verdict::Rejected(d) => Verdict::Rejected(scrub(&format!("{name} {d}"), secrets)),
        Verdict::Transient(d) => Verdict::Transient(scrub(&format!("{name} {d}"), secrets)),
        other => other,
    }
}

impl Engine {
    /// Resolves every configured channel and records what it found.
    pub(crate) fn resolve_channels(&mut self) -> Vec<Resolved> {
        let Some(n) = self.loaded.config.effective_notifications() else {
            self.channel_state.clear();
            return vec![];
        };
        let dashboard = n.dashboard_url.trim_end_matches('/').to_string();
        let mut states = BTreeMap::new();
        let mut out = vec![];
        for (id, channel) in &n.channels {
            let mut state = ChannelState::default();
            let Some(provider) = vesper_notify::provider(&channel.kind) else {
                state.error = Some("unknown channel kind".into());
                states.insert(id.clone(), state);
                continue;
            };
            match vesper_core::channels::resolve(channel, &self.secrets_root) {
                Ok(settings) => {
                    let secrets = channel
                        .settings
                        .iter()
                        .filter(|(key, value)| {
                            value.is_object() || provider.field(key).is_some_and(|f| f.secret)
                        })
                        .filter_map(|(key, _)| match settings.values().get(key)? {
                            Setting::Text(t) => Some(t.trim().to_string()),
                            Setting::List(items) => Some(items.join(",")),
                        })
                        .collect();
                    let recipient = recipient_key(id, provider, &settings);
                    state.recipient = Some(recipient.clone());
                    out.push(Resolved {
                        id: id.clone(),
                        provider,
                        settings,
                        recipient,
                        secrets,
                        dashboard: dashboard.clone(),
                    });
                }
                Err(e) => state.error = Some(e.to_string()),
            }
            states.insert(id.clone(), state);
        }
        self.channel_state = states;
        out
    }

    fn network(&self) -> Network {
        let policy = &self.loaded.config.network;
        let private_ca = policy.private_ca_file.as_ref().and_then(|file| {
            secrets::resolve(&SecretRef::File(file.clone()), &self.secrets_root)
                .map_err(|_| {
                    tracing::warn!(
                        "private CA file is unreadable; notifications trust public roots only"
                    )
                })
                .ok()
        });
        Network {
            allowed_cidrs: policy.allowed_cidrs.clone(),
            private_ca,
        }
    }

    fn rejected_now(&self, channel: &ChannelRow) -> Option<String> {
        channel
            .rejected
            .clone()
            .filter(|_| channel.rejected_revision.as_deref() == Some(self.loaded.revision.as_str()))
    }

    /// Why an undelivered dependent outage is held back for grouping, if it is:
    /// - a declared dependency is Suspect (bounded wait, so both go together);
    /// - the correlated dependency's outage was already delivered and it is
    ///   still down (the dependent is sent only if still down after the
    ///   dependency recovers, otherwise it stays history);
    /// - the dependency just recovered and the dependent's active checks have
    ///   not been rechecked since (bounded wait).
    fn dependency_hold(
        &self,
        incident: &IncidentRow,
        recipient: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<String>> {
        let wait = TimeDelta::seconds(DEPENDENCY_GROUPING_WAIT_SECONDS);
        if now - incident.opened_at < wait {
            if let Some(p) = self
                .parents(&incident.service_id)
                .into_iter()
                .find(|p| self.service_health(p, now) == vesper_core::Health::Suspect)
            {
                return Ok(Some(format!("waiting to group with dependency {p}")));
            }
        }
        let Some(parent) = &incident.correlated_with else {
            return Ok(None);
        };
        if let Some(&parent_incident) = self.open.get(parent) {
            // Only dependents that confirmed after the dependency's summary went
            // out are held; one that existed but did not fit is sent next.
            let parent_sent = self
                .store
                .delivery(parent_incident, recipient)?
                .outage_sent_at;
            if parent_sent.is_some_and(|sent| incident.opened_at > sent) {
                return Ok(Some(format!(
                    "held: dependency {parent} outage already notified; sent only if still down after it recovers"
                )));
            }
            return Ok(None);
        }
        let recovered = self
            .store
            .latest_incident(parent)?
            .and_then(|i| i.resolved_at);
        if let Some(r) = recovered.filter(|r| now - *r < wait) {
            let pending = self.service_checks(&incident.service_id).any(|(_, rt)| {
                rt.active
                    && rt.kind != vesper_core::CheckKind::Push
                    && (!rt.current || rt.row.last_checked.is_none_or(|t| t <= r))
            });
            if pending {
                return Ok(Some(format!(
                    "waiting for recheck after dependency {parent} recovered"
                )));
            }
        }
        Ok(None)
    }

    fn notifications_enabled(&self, service_id: &str) -> bool {
        find_service(&self.loaded.config, service_id).is_none_or(|(_, s)| s.notifications)
    }

    fn pending_items(
        &self,
        recipient: &str,
        max_items: usize,
        now: DateTime<Utc>,
    ) -> Result<Vec<(Kind, IncidentRow)>> {
        let reminder_every = self
            .loaded
            .config
            .effective_notifications()
            .and_then(|n| n.reminder_seconds)
            .filter(|s| *s > 0);
        let mut items = vec![];
        for incident in self.store.open_incidents()? {
            if !self.notifications_enabled(&incident.service_id)
                || self.suppression(&incident.service_id, now).is_some()
            {
                continue;
            }
            let delivery = self.store.delivery(incident.id, recipient)?;
            match delivery.outage_sent_at {
                None => {
                    if self.dependency_hold(&incident, recipient, now)?.is_none() {
                        items.push((Kind::Outage, incident));
                    }
                }
                Some(sent) => {
                    let last = delivery.last_reminder_at.unwrap_or(sent);
                    if let Some(every) = reminder_every {
                        if incident.acknowledged_at.is_none() && now >= crate::after(last, every) {
                            items.push((Kind::Reminder, incident));
                        }
                    }
                }
            }
        }
        for incident in self.store.awaiting_recovery(recipient)? {
            if self.notifications_enabled(&incident.service_id)
                && self.suppression(&incident.service_id, now).is_none()
            {
                items.push((Kind::Recovery, incident));
            }
        }
        items.sort_by(|a, b| {
            a.1.service_id
                .cmp(&b.1.service_id)
                .then(a.1.id.cmp(&b.1.id))
        });
        items.truncate(max_items);
        Ok(items)
    }

    /// Names of enabled checks that are Down on current evidence, bounded.
    fn failing_checks(&self, service_id: &str, now: DateTime<Utc>) -> String {
        let names: Vec<String> = self
            .service_checks(service_id)
            .filter(|(id, rt)| self.check_health(id, rt, now) == Health::Down)
            .filter_map(|(id, _)| {
                find_check(&self.loaded.config, id)
                    .filter(|(_, c)| c.enabled)
                    .map(|(_, c)| display_name(&c.name, id.rsplit('/').next().unwrap_or(id)))
            })
            .collect();
        let mut out = names
            .iter()
            .take(MAX_CHECK_NAMES)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if names.len() > MAX_CHECK_NAMES {
            out.push_str(&format!(" and {} more", names.len() - MAX_CHECK_NAMES));
        }
        out
    }

    fn current_name(&self, incident: &IncidentRow) -> String {
        find_service(&self.loaded.config, &incident.service_id)
            .map(|(_, s)| display_name(&s.name, &incident.service_id))
            .unwrap_or_else(|| bounded(&incident.service_name, 80))
    }

    /// What is known about an open incident's service right now. Never claims
    /// Down from stale or pre-restart evidence.
    fn open_state(&self, service_id: &str, now: DateTime<Utc>) -> &'static str {
        match self.service_health(service_id, now) {
            Health::Down => "DOWN",
            Health::Recovering => "RECOVERING",
            _ => "OPEN (no current evidence)",
        }
    }

    /// Current state of a correlated dependency, for a dependent's line.
    fn dependency_note(&self, parent: &str, now: DateTime<Utc>) -> String {
        let state = if !self.open.contains_key(parent) {
            "which has since recovered".to_string()
        } else if self.service_health(parent, now) == Health::Down {
            "which is down".to_string()
        } else {
            format!(
                "which has an open incident, currently {}",
                self.open_state(parent, now).to_lowercase()
            )
        };
        format!(
            "Depends on {}, {state} (correlation, not proven cause)",
            bounded(parent, 80)
        )
    }

    /// Composes one bounded summary from current state and returns it with
    /// the items it actually names. Items that do not fit stay pending. The
    /// text has names, times, durations and a dashboard link only; never probe
    /// evidence or credentials.
    fn compose(
        &self,
        items: &[(Kind, IncidentRow)],
        dashboard: &str,
        max_chars: usize,
        now: DateTime<Utc>,
    ) -> (Message, Vec<(Kind, i64)>) {
        let outage_services: Vec<&str> = items
            .iter()
            .filter(|(k, _)| *k == Kind::Outage)
            .map(|(_, i)| i.service_id.as_str())
            .collect();
        let nested_under = |i: &IncidentRow| {
            i.correlated_with
                .as_deref()
                .filter(|p| outage_services.contains(p))
                .map(str::to_string)
        };
        // Entries in message order: each top-level item followed by the
        // dependents grouped under it.
        let mut entries: Vec<(String, (Kind, i64))> = vec![];
        for (kind, incident) in items {
            let name = self.current_name(incident);
            let sid = bounded(&incident.service_id, 80);
            let line = match kind {
                Kind::Outage if nested_under(incident).is_some() => continue,
                Kind::Outage => {
                    let mut line = format!(
                        "{} {name} ({sid}) since {} ({})",
                        self.open_state(&incident.service_id, now),
                        fmt_display(incident.opened_at),
                        fmt_duration(now - incident.opened_at)
                    );
                    let failing = self.failing_checks(&incident.service_id, now);
                    if !failing.is_empty() {
                        line.push_str(&format!(", failing: {failing}"));
                    }
                    if let Some(p) = &incident.correlated_with {
                        line.push_str(&format!(". {}", self.dependency_note(p, now)));
                    }
                    line
                }
                Kind::Reminder => format!(
                    "STILL {} {name} ({sid}) for {}",
                    self.open_state(&incident.service_id, now),
                    fmt_duration(now - incident.opened_at)
                ),
                Kind::Recovery => {
                    let resolved = incident.resolved_at.unwrap_or(now);
                    let verb = match incident.resolution.as_deref() {
                        Some("removed") => "CLOSED (removed from configuration)",
                        Some("no_checks") => "CLOSED (no enabled checks)",
                        _ => "RECOVERED",
                    };
                    format!(
                        "{verb} {name} ({sid}) at {} after {}",
                        fmt_display(resolved),
                        fmt_duration(resolved - incident.opened_at)
                    )
                }
            };
            entries.push((line, (*kind, incident.id)));
            if *kind == Kind::Outage {
                for (_, child) in items.iter().filter(|(k, c)| {
                    *k == Kind::Outage
                        && nested_under(c).as_deref() == Some(incident.service_id.as_str())
                }) {
                    let state = match self.open_state(&child.service_id, now) {
                        "DOWN" => "Also down".to_string(),
                        other => format!("Also open ({})", other.to_lowercase()),
                    };
                    entries.push((
                        format!(
                            "  {state}, depending on {sid} (correlation, not proven cause): {} ({})",
                            self.current_name(child),
                            bounded(&child.service_id, 80)
                        ),
                        (Kind::Outage, child.id),
                    ));
                }
            }
        }
        // Whole entries only; the first always goes in because lines are
        // bounded, even if a provider's limit is tighter than one line.
        let reserve = HEADER_RESERVE + dashboard.chars().count() + "/incidents/".len() + 20;
        let budget = max_chars.saturating_sub(reserve);
        let mut used = 0;
        let mut lines = vec![];
        let mut included = vec![];
        for (line, member) in entries {
            let len = line.chars().count() + 1;
            if !lines.is_empty() && used + len > budget {
                break;
            }
            used += len;
            lines.push(line);
            included.push(member);
        }
        let count = |k: Kind| included.iter().filter(|(kind, _)| *kind == k).count();
        let mut header = vec![];
        if count(Kind::Outage) > 0 {
            header.push(format!("{} new problem(s)", count(Kind::Outage)));
        }
        if count(Kind::Reminder) > 0 {
            header.push(format!("{} still open", count(Kind::Reminder)));
        }
        if count(Kind::Recovery) > 0 {
            header.push(format!("{} resolved", count(Kind::Recovery)));
        }
        let link = match included.as_slice() {
            [(_, only)] => format!("{dashboard}/incidents/{only}"),
            _ => format!("{dashboard}/incidents"),
        };
        let status = if count(Kind::Recovery) == included.len() {
            Status::Up
        } else {
            Status::Down
        };
        let incident = match included.as_slice() {
            [(_, only)] => items
                .iter()
                .find(|(_, i)| i.id == *only)
                .map(|(_, i)| IncidentRef {
                    id: i.id,
                    service_id: i.service_id.clone(),
                    service_name: self.current_name(i),
                }),
            _ => None,
        };
        let message = Message {
            title: format!("Vesper: {}", header.join(", ")),
            lines,
            link,
            status,
            incident,
            at: now,
        };
        (message, included)
    }

    pub(crate) fn prepare_deliveries(&mut self, now: DateTime<Utc>) -> Result<Vec<Delivery>> {
        let channels = self.resolve_channels();
        let network = self.network();
        let mut out = vec![];
        for c in channels {
            let row = self.store.channel(&c.recipient)?;
            if self.rejected_now(&row).is_some()
                || row.retry_after_until.is_some_and(|t| t > now)
                || row.backoff_until.is_some_and(|t| t > now)
            {
                continue;
            }
            let max_items = if c.provider.per_incident {
                1
            } else {
                MAX_ITEMS_PER_MESSAGE
            };
            let items = self.pending_items(&c.recipient, max_items, now)?;
            if items.is_empty() {
                continue;
            }
            let (mut message, included) =
                self.compose(&items, &c.dashboard, c.provider.max_chars, now);
            if !c.provider.per_incident {
                message.incident = None;
            }
            let ids = |k: Kind| -> Vec<i64> {
                included
                    .iter()
                    .filter(|(kind, _)| *kind == k)
                    .map(|(_, id)| *id)
                    .collect()
            };
            out.push(Delivery {
                label: label(&c.id, c.provider),
                revision: self.loaded.revision.clone(),
                plan: (c.provider.build)(&c.settings, &message),
                secrets: c.secrets,
                network: network.clone(),
                outage: ids(Kind::Outage),
                recovery: ids(Kind::Recovery),
                reminder: ids(Kind::Reminder),
                recipient: c.recipient,
            });
        }
        Ok(out)
    }

    fn record_outcome(
        &mut self,
        recipient: &str,
        label: &str,
        revision: &str,
        outcome: &Verdict,
        now: DateTime<Utc>,
        transient_backoff: bool,
    ) -> Result<()> {
        let mut channel = self.store.channel(recipient)?;
        match outcome {
            Verdict::Accepted => {
                channel = ChannelRow {
                    last_success_at: Some(now),
                    ..ChannelRow::default()
                };
            }
            Verdict::RetryAfter(seconds) => {
                let until = crate::after(now, *seconds);
                channel.retry_after_until = Some(until);
                channel.last_error = Some(format!("{label} rate limit (retry_after {seconds}s)"));
                tracing::warn!("{label} rate limited delivery until {}", fmt_time(until));
            }
            Verdict::Rejected(description) => {
                channel.rejected = Some(description.clone());
                channel.rejected_revision = Some(revision.to_string());
                channel.last_error = Some(description.clone());
                tracing::error!("notification rejected: {description}");
            }
            Verdict::Transient(description) => {
                channel.last_error = Some(description.clone());
                if transient_backoff {
                    channel.failures = channel.failures.saturating_add(1);
                    channel.backoff_until = Some(now + backoff(channel.failures));
                }
                tracing::warn!("notification delivery failed: {description}");
            }
        }
        self.store.save_channel(recipient, &channel)
    }

    pub(crate) fn finish_delivery(&mut self, delivery: &Delivery, outcome: &Verdict) -> Result<()> {
        let now = self.clock.now();
        if *outcome == Verdict::Accepted {
            self.store.mark_delivered(
                &delivery.recipient,
                &delivery.label,
                now,
                &delivery.outage,
                &delivery.recovery,
                &delivery.reminder,
            )?;
        }
        self.record_outcome(
            &delivery.recipient,
            &delivery.label,
            &delivery.revision,
            outcome,
            now,
            true,
        )
    }

    fn channel_notification_state(
        &self,
        incident: &IncidentRow,
        recipient: &str,
        now: DateTime<Utc>,
    ) -> Result<String> {
        let d = self.store.delivery(incident.id, recipient)?;
        let suppression = self.suppression(&incident.service_id, now);
        Ok(
            match (incident.resolved_at, d.outage_sent_at, d.recovery_sent_at) {
                (None, Some(sent), _) => match d.last_reminder_at {
                    Some(r) => format!(
                        "outage sent {}; last reminder {}",
                        fmt_time(sent),
                        fmt_time(r)
                    ),
                    None => format!("outage sent {}", fmt_time(sent)),
                },
                (None, None, _) => {
                    match (suppression, self.dependency_hold(incident, recipient, now)?) {
                        (Some(reason), _) => format!("suppressed: {reason}"),
                        (None, Some(hold)) => hold,
                        (None, None) => "outage pending delivery".into(),
                    }
                }
                (Some(_), Some(_), Some(r)) => {
                    format!("outage and recovery sent; recovery {}", fmt_time(r))
                }
                (Some(_), Some(_), None) => match suppression {
                    Some(reason) => format!("recovery deferred: {reason}"),
                    None => "recovery pending delivery".into(),
                },
                (Some(_), None, _) => "history only: the outage was never delivered".into(),
            },
        )
    }

    /// Delivery state of an incident on every channel; prefixed by channel
    /// ID when there is more than one.
    pub(crate) fn notification_state(
        &self,
        incident: &IncidentRow,
        now: DateTime<Utc>,
    ) -> Result<String> {
        if !self.notifications_enabled(&incident.service_id) {
            return Ok("notifications disabled for this service".into());
        }
        if self.channel_state.is_empty() {
            return Ok("notifications not configured".into());
        }
        let mut states = vec![];
        for (id, state) in &self.channel_state {
            let text = match &state.recipient {
                Some(recipient) => self.channel_notification_state(incident, recipient, now)?,
                None => "awaiting channel credential check".into(),
            };
            states.push((id, text));
        }
        Ok(match states.as_slice() {
            [(_, only)] => only.clone(),
            many => many
                .iter()
                .map(|(id, text)| format!("{id}: {text}"))
                .collect::<Vec<_>>()
                .join("; "),
        })
    }

    fn channel_status(
        &self,
        state: &ChannelState,
        name: &str,
        now: DateTime<Utc>,
    ) -> Result<String> {
        if let Some(e) = &state.error {
            return Ok(format!("error: {e}"));
        }
        let Some(recipient) = &state.recipient else {
            return Ok("configured; awaiting first delivery check".into());
        };
        let channel = self.store.channel(recipient)?;
        if let Some(rejected) = self.rejected_now(&channel) {
            return Ok(format!(
                "error: {rejected}. Delivery is stopped until the configuration changes or a test message succeeds."
            ));
        }
        if let Some(until) = channel.retry_after_until.filter(|t| *t > now) {
            return Ok(format!(
                "rate limited by {name}; retrying after {}",
                fmt_time(until)
            ));
        }
        if let Some(until) = channel.backoff_until.filter(|t| *t > now) {
            return Ok(format!(
                "delivery failing ({}); retrying after {}",
                channel.last_error.unwrap_or_default(),
                fmt_time(until)
            ));
        }
        Ok(match channel.last_success_at {
            Some(t) => format!("ok; last message accepted {}", fmt_time(t)),
            None => "configured".into(),
        })
    }

    pub(crate) fn channel_views(&self, now: DateTime<Utc>) -> Result<Vec<ChannelView>> {
        let Some(n) = self.loaded.config.effective_notifications() else {
            return Ok(vec![]);
        };
        let mut out = vec![];
        for (id, channel) in &n.channels {
            let provider = vesper_notify::provider(&channel.kind);
            let name = provider.map_or(channel.kind.as_str(), |p| p.name);
            let state = self.channel_state.get(id).cloned().unwrap_or_default();
            out.push(ChannelView {
                id: id.clone(),
                kind: channel.kind.clone(),
                kind_name: name.to_string(),
                status: self.channel_status(&state, name, now)?,
            });
        }
        Ok(out)
    }
}

/// One delivery attempt per channel, sent concurrently without holding the
/// lock during network IO. Returns whether any message was attempted.
pub(crate) async fn deliver_once(engine: &Arc<Mutex<crate::Engine>>) -> bool {
    let (deliveries, transport) = {
        let mut e = engine.lock().await;
        let now = e.clock.now();
        match e.prepare_deliveries(now) {
            Ok(d) => (d, e.transport.clone()),
            Err(err) => {
                tracing::error!("could not prepare notifications: {err:#}");
                return false;
            }
        }
    };
    if deliveries.is_empty() {
        return false;
    }
    let mut set = tokio::task::JoinSet::new();
    for (index, mut delivery) in deliveries.into_iter().enumerate() {
        let transport = transport.clone();
        set.spawn(async move {
            let name = delivery.label.clone();
            let verdict = match &delivery.plan {
                Ok(plan) => {
                    let mut secrets = delivery.secrets.clone();
                    let v = execute(&*transport, plan, &delivery.network, &mut secrets).await;
                    delivery.secrets = secrets;
                    v
                }
                Err(e) => Verdict::Rejected(e.clone()),
            };
            let verdict = described(verdict, &name, &delivery.secrets);
            (index, delivery, verdict)
        });
    }
    let mut results = vec![];
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(r) => results.push(r),
            Err(_) => tracing::error!("a notification delivery task failed"),
        }
    }
    results.sort_by_key(|(index, _, _)| *index);
    let mut e = engine.lock().await;
    for (_, delivery, verdict) in results {
        if let Err(err) = e.finish_delivery(&delivery, &verdict) {
            tracing::error!("could not record notification delivery: {err:#}");
        }
    }
    true
}

/// Sends an explicit, user-requested test message on one channel.
pub(crate) async fn test_channel(
    engine: &Arc<Mutex<crate::Engine>>,
    channel_id: &str,
) -> Result<String> {
    let (plan, mut secrets, recipient, label, revision, network, transport) = {
        let mut e = engine.lock().await;
        let now = e.clock.now();
        let channels = e.resolve_channels();
        let Some(c) = channels.into_iter().find(|c| c.id == channel_id) else {
            match e
                .channel_state
                .get(channel_id)
                .and_then(|s| s.error.clone())
            {
                Some(error) => bail!("{error}"),
                None => bail!("no notification channel {}", bounded(channel_id, 80)),
            }
        };
        let row = e.store.channel(&c.recipient)?;
        let label = label(&c.id, c.provider);
        if let Some(until) = row.retry_after_until.filter(|t| *t > now) {
            bail!(
                "{label} rate limit is in effect until {}; try again later",
                fmt_time(until)
            );
        }
        let message = Message {
            title: "Vesper test message".into(),
            lines: vec![format!(
                "Sent {}. Notifications reach this channel.",
                fmt_display(now)
            )],
            link: c.dashboard.clone(),
            status: Status::Test,
            incident: None,
            at: now,
        };
        (
            (c.provider.build)(&c.settings, &message),
            c.secrets,
            c.recipient,
            label,
            e.loaded.revision.clone(),
            e.network(),
            e.transport.clone(),
        )
    };
    let verdict = match &plan {
        Ok(plan) => execute(&*transport, plan, &network, &mut secrets).await,
        Err(e) => Verdict::Rejected(e.clone()),
    };
    let verdict = described(verdict, &label, &secrets);
    let mut e = engine.lock().await;
    let now = e.clock.now();
    e.record_outcome(&recipient, &label, &revision, &verdict, now, false)?;
    e.wake.notify_one();
    match verdict {
        Verdict::Accepted => Ok(format!(
            "{label} accepted the test message (acceptance is not a read receipt)"
        )),
        Verdict::RetryAfter(s) => {
            bail!("{label} rate limited the request; retry after {s} seconds")
        }
        Verdict::Rejected(d) => Err(anyhow!("{d}")),
        Verdict::Transient(d) => bail!("{d}; the message may or may not have been delivered"),
    }
}
