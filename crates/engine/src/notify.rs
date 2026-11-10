//! Notification decisions and delivery. Delivery is pull-based: every attempt
//! is composed from the current incident state and the persisted per-recipient
//! delivery marks, so a delayed or rate-limited channel sends one current
//! summary instead of a burst of obsolete events.

use crate::store::{fmt_time, ChannelRow, IncidentRow};
use crate::telegram::{classify, Outcome, TelegramRequest};
use crate::DEPENDENCY_GROUPING_WAIT_SECONDS;
use crate::{bounded, display_name, find_check, find_service, Engine, State};
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Duration as TimeDelta, Utc};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;
use vesper_core::secrets;

const MAX_ITEMS_PER_MESSAGE: usize = 25;
const MAX_MESSAGE_CHARS: usize = 3800;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Outage,
    Reminder,
    Recovery,
}

pub(crate) struct Delivery {
    recipient: String,
    revision: String,
    request: TelegramRequest,
    outage: Vec<i64>,
    recovery: Vec<i64>,
    reminder: Vec<i64>,
}

struct Credentials {
    token: String,
    chat: String,
    recipient: String,
    dashboard: String,
}

fn recipient_key(chat: &str) -> String {
    let digest = Sha256::digest(chat.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("telegram:{hex}")
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

impl Engine {
    fn credentials(&mut self) -> Option<Credentials> {
        let telegram = self.loaded.config.telegram.clone();
        let Some(telegram) = telegram else {
            self.recipient = None;
            self.credential_error = None;
            return None;
        };
        let token = secrets::resolve(&telegram.bot_token, &self.secrets_root);
        let chat = secrets::resolve(&telegram.chat_id, &self.secrets_root);
        match (token, chat) {
            (Ok(token), Ok(chat)) => {
                let token = token.trim().to_string();
                let chat = chat.trim().to_string();
                let recipient = recipient_key(&chat);
                self.recipient = Some(recipient.clone());
                self.credential_error = None;
                Some(Credentials {
                    token,
                    chat,
                    recipient,
                    dashboard: telegram.dashboard_url.trim_end_matches('/').to_string(),
                })
            }
            (Err(_), _) => {
                self.credential_error =
                    Some("could not resolve the Telegram bot token secret".into());
                None
            }
            (_, Err(_)) => {
                self.credential_error =
                    Some("could not resolve the Telegram chat ID secret".into());
                None
            }
        }
    }

    fn rejected_now(&self, channel: &ChannelRow) -> Option<String> {
        channel
            .rejected
            .clone()
            .filter(|_| channel.rejected_revision.as_deref() == Some(self.loaded.revision.as_str()))
    }

    /// A Suspect parent the undelivered outage should wait for, if any.
    fn grouping_wait(&self, incident: &IncidentRow, now: DateTime<Utc>) -> Option<String> {
        if now - incident.opened_at >= TimeDelta::seconds(DEPENDENCY_GROUPING_WAIT_SECONDS) {
            return None;
        }
        self.parents(&incident.service_id)
            .into_iter()
            .find(|p| self.service_health(p, now) == vesper_core::Health::Suspect)
    }

    fn notifications_enabled(&self, service_id: &str) -> bool {
        find_service(&self.loaded.config, service_id).is_none_or(|(_, s)| s.notifications)
    }

    fn pending_items(
        &self,
        recipient: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<(Kind, IncidentRow)>> {
        let reminder_every = self
            .loaded
            .config
            .telegram
            .as_ref()
            .and_then(|t| t.reminder_seconds)
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
                    if self.grouping_wait(&incident, now).is_none() {
                        items.push((Kind::Outage, incident));
                    }
                }
                Some(sent) => {
                    let last = delivery.last_reminder_at.unwrap_or(sent);
                    if let Some(every) = reminder_every {
                        if incident.acknowledged_at.is_none() && now - last >= crate::secs(every) {
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
        items.truncate(MAX_ITEMS_PER_MESSAGE);
        Ok(items)
    }

    fn failing_checks(&self, service_id: &str) -> Vec<String> {
        self.service_checks(service_id)
            .filter(|(_, rt)| matches!(rt.row.state, State::Down | State::Recovering))
            .filter_map(|(id, _)| {
                find_check(&self.loaded.config, id)
                    .filter(|(_, c)| c.enabled)
                    .map(|(_, c)| display_name(&c.name, id.rsplit('/').next().unwrap_or(id)))
            })
            .collect()
    }

    fn current_name(&self, incident: &IncidentRow) -> String {
        find_service(&self.loaded.config, &incident.service_id)
            .map(|(_, s)| display_name(&s.name, &incident.service_id))
            .unwrap_or_else(|| bounded(&incident.service_name, 80))
    }

    /// Composes one bounded summary. Contains names, times, durations and a
    /// dashboard link only; never probe evidence or credentials.
    fn compose(
        &self,
        items: &[(Kind, IncidentRow)],
        dashboard: &str,
        now: DateTime<Utc>,
    ) -> String {
        let outages: Vec<&IncidentRow> = items
            .iter()
            .filter(|(k, _)| *k == Kind::Outage)
            .map(|(_, i)| i)
            .collect();
        let grouped_parent = |i: &IncidentRow| {
            i.correlated_with
                .as_ref()
                .filter(|p| outages.iter().any(|o| &&o.service_id == p))
                .cloned()
        };
        let count = |k: Kind| items.iter().filter(|(kind, _)| *kind == k).count();
        let mut header = vec![];
        if count(Kind::Outage) > 0 {
            header.push(format!("{} down", count(Kind::Outage)));
        }
        if count(Kind::Reminder) > 0 {
            header.push(format!("{} still down", count(Kind::Reminder)));
        }
        if count(Kind::Recovery) > 0 {
            header.push(format!("{} resolved", count(Kind::Recovery)));
        }
        let mut lines = vec![format!("Vesper: {}", header.join(", "))];
        for (kind, incident) in items {
            let name = self.current_name(incident);
            let sid = &incident.service_id;
            match kind {
                Kind::Outage if grouped_parent(incident).is_some() => {}
                Kind::Outage => {
                    let mut line = format!(
                        "DOWN {name} ({sid}) since {} ({})",
                        fmt_display(incident.opened_at),
                        fmt_duration(now - incident.opened_at)
                    );
                    let failing = self.failing_checks(sid);
                    if !failing.is_empty() {
                        line.push_str(&format!(", failing: {}", failing.join(", ")));
                    }
                    if let Some(p) = &incident.correlated_with {
                        line.push_str(&format!(
                            ". Depends on {p}, which is down (correlation, not proven cause)"
                        ));
                    }
                    lines.push(line);
                    let children: Vec<String> = outages
                        .iter()
                        .filter(|c| grouped_parent(c).as_deref() == Some(sid.as_str()))
                        .map(|c| format!("{} ({})", self.current_name(c), c.service_id))
                        .collect();
                    if !children.is_empty() {
                        lines.push(format!(
                            "  Also down, depending on {sid} (correlation, not proven cause): {}",
                            children.join(", ")
                        ));
                    }
                }
                Kind::Reminder => lines.push(format!(
                    "STILL DOWN {name} ({sid}) for {}",
                    fmt_duration(now - incident.opened_at)
                )),
                Kind::Recovery => {
                    let resolved = incident.resolved_at.unwrap_or(now);
                    let verb = match incident.resolution.as_deref() {
                        Some("removed") => "CLOSED (removed from configuration)",
                        Some("no_checks") => "CLOSED (no enabled checks)",
                        _ => "RECOVERED",
                    };
                    lines.push(format!(
                        "{verb} {name} ({sid}) at {} after {}",
                        fmt_display(resolved),
                        fmt_duration(resolved - incident.opened_at)
                    ));
                }
            }
        }
        let link = match items {
            [(_, only)] => format!("{dashboard}/incidents/{}", only.id),
            _ => format!("{dashboard}/incidents"),
        };
        let mut text = String::new();
        for line in lines {
            if text.chars().count() + line.chars().count() + link.len() + 8 > MAX_MESSAGE_CHARS {
                text.push_str("…\n");
                break;
            }
            text.push_str(&line);
            text.push('\n');
        }
        text.push_str(&link);
        text
    }

    pub(crate) fn prepare_delivery(&mut self, now: DateTime<Utc>) -> Result<Option<Delivery>> {
        let Some(creds) = self.credentials() else {
            return Ok(None);
        };
        let channel = self.store.channel(&creds.recipient)?;
        if self.rejected_now(&channel).is_some()
            || channel.retry_after_until.is_some_and(|t| t > now)
            || channel.backoff_until.is_some_and(|t| t > now)
        {
            return Ok(None);
        }
        let items = self.pending_items(&creds.recipient, now)?;
        if items.is_empty() {
            return Ok(None);
        }
        let text = self.compose(&items, &creds.dashboard, now);
        let ids = |k: Kind| -> Vec<i64> {
            items
                .iter()
                .filter(|(kind, _)| *kind == k)
                .map(|(_, i)| i.id)
                .collect()
        };
        Ok(Some(Delivery {
            recipient: creds.recipient,
            revision: self.loaded.revision.clone(),
            request: TelegramRequest {
                bot_token: creds.token,
                chat_id: creds.chat,
                text,
            },
            outage: ids(Kind::Outage),
            recovery: ids(Kind::Recovery),
            reminder: ids(Kind::Reminder),
        }))
    }

    fn record_outcome(
        &mut self,
        recipient: &str,
        revision: &str,
        outcome: &Outcome,
        now: DateTime<Utc>,
        transient_backoff: bool,
    ) -> Result<()> {
        let mut channel = self.store.channel(recipient)?;
        match outcome {
            Outcome::Accepted => {
                channel = ChannelRow {
                    last_success_at: Some(now),
                    ..ChannelRow::default()
                };
            }
            Outcome::RetryAfter(seconds) => {
                let until = now + crate::secs(*seconds);
                channel.retry_after_until = Some(until);
                channel.last_error = Some(format!("Telegram rate limit (retry_after {seconds}s)"));
                tracing::warn!("Telegram rate limited delivery until {}", fmt_time(until));
            }
            Outcome::Rejected(description) => {
                channel.rejected = Some(description.clone());
                channel.rejected_revision = Some(revision.to_string());
                channel.last_error = Some(description.clone());
                tracing::error!("Telegram rejected delivery: {description}");
            }
            Outcome::Transient(description) => {
                channel.last_error = Some(description.clone());
                if transient_backoff {
                    channel.failures = channel.failures.saturating_add(1);
                    channel.backoff_until = Some(now + backoff(channel.failures));
                }
                tracing::warn!("Telegram delivery failed: {description}");
            }
        }
        self.store.save_channel(recipient, &channel)
    }

    pub(crate) fn finish_delivery(&mut self, delivery: &Delivery, outcome: &Outcome) -> Result<()> {
        let now = self.clock.now();
        if *outcome == Outcome::Accepted {
            self.store.mark_delivered(
                &delivery.recipient,
                now,
                &delivery.outage,
                &delivery.recovery,
                &delivery.reminder,
            )?;
        }
        self.record_outcome(&delivery.recipient, &delivery.revision, outcome, now, true)
    }

    pub(crate) fn notification_state(
        &self,
        incident: &IncidentRow,
        now: DateTime<Utc>,
    ) -> Result<String> {
        if !self.notifications_enabled(&incident.service_id) {
            return Ok("notifications disabled for this service".into());
        }
        if self.loaded.config.telegram.is_none() {
            return Ok("Telegram not configured".into());
        }
        let Some(recipient) = &self.recipient else {
            return Ok("awaiting Telegram credential check".into());
        };
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
                (None, None, _) => match (suppression, self.grouping_wait(incident, now)) {
                    (Some(reason), _) => format!("suppressed: {reason}"),
                    (None, Some(parent)) => format!("waiting to group with dependency {parent}"),
                    (None, None) => "outage pending delivery".into(),
                },
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

    pub(crate) fn telegram_status(&self, now: DateTime<Utc>) -> Result<String> {
        if self.loaded.config.telegram.is_none() {
            return Ok("not configured".into());
        }
        if let Some(e) = &self.credential_error {
            return Ok(format!("error: {e}"));
        }
        let Some(recipient) = &self.recipient else {
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
                "rate limited by Telegram; retrying after {}",
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
}

/// One delivery attempt without holding the lock during network IO. Returns
/// whether a message was attempted.
pub(crate) async fn deliver_once(engine: &Arc<Mutex<crate::Engine>>) -> bool {
    let (delivery, transport) = {
        let mut e = engine.lock().await;
        let now = e.clock.now();
        match e.prepare_delivery(now) {
            Ok(Some(d)) => (d, e.transport.clone()),
            Ok(None) => return false,
            Err(err) => {
                tracing::error!("could not prepare notifications: {err:#}");
                return false;
            }
        }
    };
    let result = transport.send_message(&delivery.request).await;
    let outcome = classify(
        result,
        &[&delivery.request.bot_token, &delivery.request.chat_id],
    );
    let mut e = engine.lock().await;
    if let Err(err) = e.finish_delivery(&delivery, &outcome) {
        tracing::error!("could not record notification delivery: {err:#}");
    }
    true
}

pub(crate) async fn test_telegram(engine: &Arc<Mutex<crate::Engine>>) -> Result<String> {
    let (request, recipient, revision, transport) = {
        let mut e = engine.lock().await;
        let now = e.clock.now();
        if e.loaded.config.telegram.is_none() {
            bail!("Telegram is not configured");
        }
        let creds = e
            .credentials()
            .ok_or_else(|| anyhow!(e.credential_error.clone().unwrap_or_default()))?;
        let channel = e.store.channel(&creds.recipient)?;
        if let Some(until) = channel.retry_after_until.filter(|t| *t > now) {
            bail!(
                "Telegram rate limit is in effect until {}; try again later",
                fmt_time(until)
            );
        }
        let text = format!(
            "Vesper test message, sent {}. Notifications reach this chat.\n{}",
            fmt_display(now),
            creds.dashboard
        );
        (
            TelegramRequest {
                bot_token: creds.token,
                chat_id: creds.chat,
                text,
            },
            creds.recipient,
            e.loaded.revision.clone(),
            e.transport.clone(),
        )
    };
    let result = transport.send_message(&request).await;
    let outcome = classify(result, &[&request.bot_token, &request.chat_id]);
    let mut e = engine.lock().await;
    let now = e.clock.now();
    e.record_outcome(&recipient, &revision, &outcome, now, false)?;
    e.wake.notify_one();
    match outcome {
        Outcome::Accepted => {
            Ok("Telegram accepted the test message (acceptance is not a read receipt)".into())
        }
        Outcome::RetryAfter(s) => {
            bail!("Telegram rate limited the request; retry after {s} seconds")
        }
        Outcome::Rejected(d) => bail!("Telegram rejected the request: {d}"),
        Outcome::Transient(d) => bail!("{d}; the message may or may not have been delivered"),
    }
}
