//! SQLite operational persistence: check state, samples, incidents, delivery
//! awareness, Telegram channel state, silences and audit events.

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeMap;
use std::path::Path;
use vesper_core::{IncidentEvent, Sample};

const SAMPLES_KEPT_PER_CHECK: i64 = 500;

pub(crate) fn fmt_time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn opt_time(s: Option<String>) -> Option<DateTime<Utc>> {
    s.as_deref().and_then(parse_time)
}

/// Threshold state machine position of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum State {
    #[default]
    Unknown,
    Healthy,
    Suspect,
    Down,
    Recovering,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Unknown => "unknown",
            State::Healthy => "healthy",
            State::Suspect => "suspect",
            State::Down => "down",
            State::Recovering => "recovering",
        }
    }
    fn parse(s: &str) -> State {
        match s {
            "healthy" => State::Healthy,
            "suspect" => State::Suspect,
            "down" => State::Down,
            "recovering" => State::Recovering,
            _ => State::Unknown,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CheckRow {
    pub state: State,
    pub failures: u32,
    pub successes: u32,
    pub last_checked: Option<DateTime<Utc>>,
    pub latency_ms: Option<u64>,
    pub message: String,
    pub last_heartbeat: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub(crate) struct IncidentRow {
    pub id: i64,
    pub service_id: String,
    pub service_name: String,
    pub opened_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution: Option<String>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub correlated_with: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DeliveryRow {
    pub outage_sent_at: Option<DateTime<Utc>>,
    pub recovery_sent_at: Option<DateTime<Utc>>,
    pub last_reminder_at: Option<DateTime<Utc>>,
}

/// Durable per-recipient Telegram channel state.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChannelRow {
    /// Telegram-requested `retry_after` deadline.
    pub retry_after_until: Option<DateTime<Utc>>,
    /// Local exponential backoff after transient failures.
    pub backoff_until: Option<DateTime<Utc>>,
    pub failures: u32,
    pub last_error: Option<String>,
    /// Permission/authentication rejection, blocking delivery for this revision.
    pub rejected: Option<String>,
    pub rejected_revision: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
}

pub(crate) struct Store {
    conn: Connection,
}

const INCIDENT_COLUMNS: &str = "id, service_id, service_name, opened_at, resolved_at, resolution, acknowledged_at, correlated_with";

fn incident_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<IncidentRow> {
    Ok(IncidentRow {
        id: r.get(0)?,
        service_id: r.get(1)?,
        service_name: r.get(2)?,
        opened_at: opt_time(Some(r.get(3)?)).unwrap_or_default(),
        resolved_at: opt_time(r.get(4)?),
        resolution: r.get(5)?,
        acknowledged_at: opt_time(r.get(6)?),
        correlated_with: r.get(7)?,
    })
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path).context("could not open the Vesper database")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // FULL keeps delivery marks durable across power loss so restarts do
        // not resend already accepted notifications.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;",
        )?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 1 {
            anyhow::bail!("database schema is newer than this Vesper version");
        }
        if version < 1 {
            self.conn.execute_batch(
                "BEGIN;
                CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS check_state (
                    check_id TEXT PRIMARY KEY,
                    state TEXT NOT NULL,
                    failures INTEGER NOT NULL,
                    successes INTEGER NOT NULL,
                    last_checked TEXT,
                    latency_ms INTEGER,
                    message TEXT NOT NULL,
                    last_heartbeat TEXT
                );
                CREATE TABLE IF NOT EXISTS samples (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    check_id TEXT NOT NULL,
                    at TEXT NOT NULL,
                    healthy INTEGER NOT NULL,
                    latency_ms INTEGER NOT NULL,
                    message TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS samples_check ON samples (check_id, id);
                CREATE TABLE IF NOT EXISTS incidents (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    service_id TEXT NOT NULL,
                    service_name TEXT NOT NULL,
                    opened_at TEXT NOT NULL,
                    resolved_at TEXT,
                    resolution TEXT,
                    acknowledged_at TEXT,
                    correlated_with TEXT
                );
                CREATE INDEX IF NOT EXISTS incidents_open ON incidents (resolved_at, service_id);
                CREATE TABLE IF NOT EXISTS incident_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    incident_id INTEGER NOT NULL REFERENCES incidents (id),
                    at TEXT NOT NULL,
                    message TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS incident_events_incident ON incident_events (incident_id, id);
                CREATE TABLE IF NOT EXISTS deliveries (
                    incident_id INTEGER NOT NULL REFERENCES incidents (id),
                    recipient TEXT NOT NULL,
                    outage_sent_at TEXT,
                    recovery_sent_at TEXT,
                    last_reminder_at TEXT,
                    PRIMARY KEY (incident_id, recipient)
                );
                CREATE TABLE IF NOT EXISTS telegram_channel (
                    recipient TEXT PRIMARY KEY,
                    retry_after_until TEXT,
                    backoff_until TEXT,
                    failures INTEGER NOT NULL DEFAULT 0,
                    last_error TEXT,
                    rejected TEXT,
                    rejected_revision TEXT,
                    last_success_at TEXT
                );
                CREATE TABLE IF NOT EXISTS silences (service_id TEXT PRIMARY KEY, until TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS audit (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    at TEXT NOT NULL,
                    action TEXT NOT NULL,
                    target TEXT NOT NULL
                );
                PRAGMA user_version = 1;
                COMMIT;",
            )?;
        }
        Ok(())
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn check_states(&self) -> Result<BTreeMap<String, CheckRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT check_id, state, failures, successes, last_checked, latency_ms, message, last_heartbeat FROM check_state",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                CheckRow {
                    state: State::parse(&r.get::<_, String>(1)?),
                    failures: r.get(2)?,
                    successes: r.get(3)?,
                    last_checked: opt_time(r.get(4)?),
                    latency_ms: r.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u64),
                    message: r.get(6)?,
                    last_heartbeat: opt_time(r.get(7)?),
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn save_check(&self, id: &str, row: &CheckRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO check_state (check_id, state, failures, successes, last_checked, latency_ms, message, last_heartbeat)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (check_id) DO UPDATE SET state = excluded.state, failures = excluded.failures,
               successes = excluded.successes, last_checked = excluded.last_checked,
               latency_ms = excluded.latency_ms, message = excluded.message,
               last_heartbeat = excluded.last_heartbeat",
            params![
                id,
                row.state.as_str(),
                row.failures,
                row.successes,
                row.last_checked.map(fmt_time),
                row.latency_ms.map(|v| v as i64),
                row.message,
                row.last_heartbeat.map(fmt_time),
            ],
        )?;
        Ok(())
    }

    pub fn delete_check(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM check_state WHERE check_id = ?1", [id])?;
        Ok(())
    }

    /// Persists a sample and the resulting check state atomically.
    pub fn record_sample(&self, id: &str, row: &CheckRow, sample: &Sample) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO samples (check_id, at, healthy, latency_ms, message) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, sample.at, sample.healthy, sample.latency_ms as i64, sample.message],
        )?;
        tx.execute(
            "DELETE FROM samples WHERE check_id = ?1 AND id <= (
               SELECT id FROM samples WHERE check_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
            params![id, SAMPLES_KEPT_PER_CHECK],
        )?;
        self.save_check(id, row)?;
        tx.commit()?;
        Ok(())
    }

    /// Most recent samples, oldest first.
    pub fn samples(&self, id: &str, limit: usize) -> Result<Vec<Sample>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT at, healthy, latency_ms, message FROM samples WHERE check_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let mut out = stmt
            .query_map(params![id, limit as i64], |r| {
                Ok(Sample {
                    at: r.get(0)?,
                    healthy: r.get(1)?,
                    latency_ms: r.get::<_, i64>(2)?.max(0) as u64,
                    message: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        out.reverse();
        Ok(out)
    }

    pub fn open_incident(
        &self,
        service_id: &str,
        service_name: &str,
        at: DateTime<Utc>,
        correlated_with: Option<&str>,
        event: &str,
    ) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO incidents (service_id, service_name, opened_at, correlated_with) VALUES (?1, ?2, ?3, ?4)",
            params![service_id, service_name, fmt_time(at), correlated_with],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO incident_events (incident_id, at, message) VALUES (?1, ?2, ?3)",
            params![id, fmt_time(at), event],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn resolve_incident(
        &self,
        id: i64,
        at: DateTime<Utc>,
        resolution: &str,
        event: &str,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE incidents SET resolved_at = ?2, resolution = ?3 WHERE id = ?1 AND resolved_at IS NULL",
            params![id, fmt_time(at), resolution],
        )?;
        tx.execute(
            "INSERT INTO incident_events (incident_id, at, message) VALUES (?1, ?2, ?3)",
            params![id, fmt_time(at), event],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_correlation(&self, id: i64, with: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE incidents SET correlated_with = ?2 WHERE id = ?1",
            params![id, with],
        )?;
        Ok(())
    }

    pub fn set_acknowledged(&self, id: i64, at: Option<DateTime<Utc>>) -> Result<()> {
        self.conn.execute(
            "UPDATE incidents SET acknowledged_at = ?2 WHERE id = ?1",
            params![id, at.map(fmt_time)],
        )?;
        Ok(())
    }

    pub fn add_event(&self, incident_id: i64, at: DateTime<Utc>, message: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO incident_events (incident_id, at, message) VALUES (?1, ?2, ?3)",
            params![incident_id, fmt_time(at), message],
        )?;
        Ok(())
    }

    pub fn incident(&self, id: i64) -> Result<Option<IncidentRow>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {INCIDENT_COLUMNS} FROM incidents WHERE id = ?1"),
                [id],
                incident_from_row,
            )
            .optional()?)
    }

    pub fn latest_incident(&self, service_id: &str) -> Result<Option<IncidentRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {INCIDENT_COLUMNS} FROM incidents WHERE service_id = ?1 ORDER BY id DESC LIMIT 1"
                ),
                [service_id],
                incident_from_row,
            )
            .optional()?)
    }

    pub fn open_incidents(&self) -> Result<Vec<IncidentRow>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {INCIDENT_COLUMNS} FROM incidents WHERE resolved_at IS NULL ORDER BY id"
        ))?;
        let rows = stmt.query_map([], incident_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Open incidents first (newest first), then recently resolved ones.
    pub fn recent_incidents(&self, resolved_limit: usize) -> Result<Vec<IncidentRow>> {
        let mut out = self.open_incidents()?;
        out.reverse();
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {INCIDENT_COLUMNS} FROM incidents WHERE resolved_at IS NOT NULL ORDER BY resolved_at DESC, id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([resolved_limit as i64], incident_from_row)?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Resolved incidents whose outage reached `recipient` but whose recovery has not.
    pub fn awaiting_recovery(&self, recipient: &str) -> Result<Vec<IncidentRow>> {
        let columns = INCIDENT_COLUMNS
            .split(", ")
            .map(|c| format!("i.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {columns} FROM incidents i JOIN deliveries d ON d.incident_id = i.id
             WHERE i.resolved_at IS NOT NULL AND d.recipient = ?1
               AND d.outage_sent_at IS NOT NULL AND d.recovery_sent_at IS NULL
             ORDER BY i.id"
        ))?;
        let rows = stmt.query_map([recipient], incident_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Most recent events, oldest first.
    pub fn events(&self, incident_id: i64, limit: usize) -> Result<Vec<IncidentEvent>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT at, message FROM incident_events WHERE incident_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let mut out = stmt
            .query_map(params![incident_id, limit as i64], |r| {
                Ok(IncidentEvent {
                    at: r.get(0)?,
                    message: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        out.reverse();
        Ok(out)
    }

    pub fn delivery(&self, incident_id: i64, recipient: &str) -> Result<DeliveryRow> {
        Ok(self
            .conn
            .query_row(
                "SELECT outage_sent_at, recovery_sent_at, last_reminder_at FROM deliveries WHERE incident_id = ?1 AND recipient = ?2",
                params![incident_id, recipient],
                |r| {
                    Ok(DeliveryRow {
                        outage_sent_at: opt_time(r.get(0)?),
                        recovery_sent_at: opt_time(r.get(1)?),
                        last_reminder_at: opt_time(r.get(2)?),
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Records accepted deliveries for one message atomically.
    pub fn mark_delivered(
        &self,
        recipient: &str,
        at: DateTime<Utc>,
        outage: &[i64],
        recovery: &[i64],
        reminder: &[i64],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let at_s = fmt_time(at);
        for (ids, column, event) in [
            (
                outage,
                "outage_sent_at",
                "Outage notification accepted by Telegram",
            ),
            (
                recovery,
                "recovery_sent_at",
                "Recovery notification accepted by Telegram",
            ),
            (
                reminder,
                "last_reminder_at",
                "Reminder accepted by Telegram",
            ),
        ] {
            for id in ids {
                tx.execute(
                    &format!(
                        "INSERT INTO deliveries (incident_id, recipient, {column}) VALUES (?1, ?2, ?3)
                         ON CONFLICT (incident_id, recipient) DO UPDATE SET {column} = excluded.{column}"
                    ),
                    params![id, recipient, at_s],
                )?;
                tx.execute(
                    "INSERT INTO incident_events (incident_id, at, message) VALUES (?1, ?2, ?3)",
                    params![id, at_s, event],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn channel(&self, recipient: &str) -> Result<ChannelRow> {
        Ok(self
            .conn
            .query_row(
                "SELECT retry_after_until, backoff_until, failures, last_error, rejected, rejected_revision, last_success_at
                 FROM telegram_channel WHERE recipient = ?1",
                [recipient],
                |r| {
                    Ok(ChannelRow {
                        retry_after_until: opt_time(r.get(0)?),
                        backoff_until: opt_time(r.get(1)?),
                        failures: r.get(2)?,
                        last_error: r.get(3)?,
                        rejected: r.get(4)?,
                        rejected_revision: r.get(5)?,
                        last_success_at: opt_time(r.get(6)?),
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    pub fn save_channel(&self, recipient: &str, c: &ChannelRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO telegram_channel (recipient, retry_after_until, backoff_until, failures, last_error, rejected, rejected_revision, last_success_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (recipient) DO UPDATE SET retry_after_until = excluded.retry_after_until,
               backoff_until = excluded.backoff_until, failures = excluded.failures,
               last_error = excluded.last_error, rejected = excluded.rejected,
               rejected_revision = excluded.rejected_revision, last_success_at = excluded.last_success_at",
            params![
                recipient,
                c.retry_after_until.map(fmt_time),
                c.backoff_until.map(fmt_time),
                c.failures,
                c.last_error,
                c.rejected,
                c.rejected_revision,
                c.last_success_at.map(fmt_time),
            ],
        )?;
        Ok(())
    }

    pub fn silences(&self) -> Result<BTreeMap<String, DateTime<Utc>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT service_id, until FROM silences")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (id, until) = row?;
            if let Some(t) = parse_time(&until) {
                out.insert(id, t);
            }
        }
        Ok(out)
    }

    pub fn set_silence(&self, service_id: &str, until: Option<DateTime<Utc>>) -> Result<()> {
        match until {
            Some(t) => self.conn.execute(
                "INSERT INTO silences (service_id, until) VALUES (?1, ?2)
                 ON CONFLICT (service_id) DO UPDATE SET until = excluded.until",
                params![service_id, fmt_time(t)],
            )?,
            None => self
                .conn
                .execute("DELETE FROM silences WHERE service_id = ?1", [service_id])?,
        };
        Ok(())
    }

    pub fn audit(&self, at: DateTime<Utc>, action: &str, target: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit (at, action, target) VALUES (?1, ?2, ?3)",
            params![fmt_time(at), action, target],
        )?;
        Ok(())
    }
}
