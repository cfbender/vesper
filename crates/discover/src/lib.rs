//! Opt-in Docker label discovery. When `VESPER_DISCOVER_DOCKER` is set,
//! `vesper serve` runs [`run`] beside the dashboard: it reads container labels
//! from one Docker endpoint per machine and writes one generated YAML file per
//! discovered service, which the dashboard reloads like any other edit.
//! `vesper discover` runs one reconciliation from the command line. See
//! `docs/discovery.md`.

pub mod docker;
pub mod labels;
pub mod reconcile;

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;
use vesper_core::config::{self, FileChange};
use vesper_core::{secrets, SecretRef};

pub const USAGE: &str = "vesper discover [--dry-run]
  Reads vesper.* labels from Docker once and brings the generated service
  files under VESPER_CONFIG_DIR in step with them; exits nonzero if anything
  failed. Settings labels on Vesper's own container, such as
  vesper.network.allowed_cidrs, are written to _labels.yaml, below every
  other file. `vesper serve` does the same continuously when
  VESPER_DISCOVER_DOCKER is set. See docs/discovery.md.
  --dry-run  show what would change with the file contents; write nothing
Environment:
  VESPER_DISCOVER_DOCKER        machine=endpoint[,machine=endpoint...], endpoints
                                unix:///path or tcp://host:port; unset means
                                discovery is off
  VESPER_DISCOVER_INTERVAL      seconds between scans, 5 to 3600 (30)
  VESPER_DISCOVER_REMOVE_AFTER  seconds a service may go undeclared before its
                                file is removed, or `never` (3600)
  VESPER_DISCOVER_ALLOWED_SECRETS  JSON map of machine IDs to secret references;
                                   empty by default (see docs/discovery.md)
  VESPER_CONFIG_DIR             configuration root (./config)";

#[derive(Debug, Clone)]
pub struct Settings {
    pub config_dir: PathBuf,
    pub sources: Vec<(String, docker::Endpoint)>,
    pub interval: Duration,
    pub remove_after: Option<Duration>,
    pub allowed_secrets: BTreeMap<String, Vec<SecretRef>>,
    /// The ID, or ID prefix, of the container Vesper runs in, whose
    /// configuration labels become `_labels.yaml`.
    pub own_container: Option<String>,
}

impl Settings {
    /// `None` when `VESPER_DISCOVER_DOCKER` is unset: discovery is off.
    pub fn from_env() -> Result<Option<Self>> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let Some(docker) = var("VESPER_DISCOVER_DOCKER") else {
            return Ok(None);
        };
        let mut sources: Vec<(String, docker::Endpoint)> = Vec::new();
        for pair in docker.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (machine, endpoint) = pair
                .split_once('=')
                .context("VESPER_DISCOVER_DOCKER entries are machine=endpoint")?;
            let machine = machine.trim();
            if !config::is_valid_id(machine) {
                bail!("VESPER_DISCOVER_DOCKER: `{machine}` is not a valid machine ID");
            }
            if sources.iter().any(|(m, _)| m == machine) {
                bail!("VESPER_DISCOVER_DOCKER: machine {machine} is listed twice; use one Docker endpoint per machine");
            }
            let endpoint = endpoint
                .trim()
                .parse()
                .with_context(|| format!("VESPER_DISCOVER_DOCKER: machine {machine}"))?;
            sources.push((machine.to_string(), endpoint));
        }
        if sources.is_empty() {
            bail!("VESPER_DISCOVER_DOCKER lists no machine=endpoint pairs");
        }
        let allowed_secrets: BTreeMap<String, Vec<SecretRef>> =
            serde_json::from_str(&var("VESPER_DISCOVER_ALLOWED_SECRETS").unwrap_or_else(|| "{}".into()))
                .map_err(|_| anyhow::anyhow!("VESPER_DISCOVER_ALLOWED_SECRETS must be a JSON map of machine IDs to lists of {{file: name}} or {{env: NAME}} references"))?;
        for (machine, refs) in &allowed_secrets {
            if !sources.iter().any(|(m, _)| m == machine) {
                bail!("VESPER_DISCOVER_ALLOWED_SECRETS names a machine not listed in VESPER_DISCOVER_DOCKER");
            }
            if refs.iter().any(|s| match s {
                SecretRef::File(name) => !secrets::is_valid_file_name(name),
                SecretRef::Env(name) => !secrets::is_valid_env_name(name),
            }) {
                bail!("VESPER_DISCOVER_ALLOWED_SECRETS contains an invalid secret reference");
            }
        }
        let interval = match var("VESPER_DISCOVER_INTERVAL") {
            None => 30,
            Some(v) => v
                .trim()
                .parse()
                .ok()
                .filter(|s| (5..=3600).contains(s))
                .context("VESPER_DISCOVER_INTERVAL must be 5 to 3600 seconds")?,
        };
        let remove_after =
            match var("VESPER_DISCOVER_REMOVE_AFTER")
                .as_deref()
                .map(str::trim)
            {
                None => Some(3600),
                Some("never") => None,
                Some(v) => Some(v.parse().ok().filter(|s| *s <= 365 * 86400).context(
                    "VESPER_DISCOVER_REMOVE_AFTER must be a number of seconds or `never`",
                )?),
            };
        Ok(Some(Settings {
            config_dir: std::env::var_os("VESPER_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| "./config".into()),
            sources,
            interval: Duration::from_secs(interval),
            remove_after: remove_after.map(Duration::from_secs),
            allowed_secrets,
            own_container: docker::own_container_id(),
        }))
    }

    /// The configured sources, as `machine=endpoint` pairs for messages.
    pub fn describe(&self) -> String {
        let machines: Vec<String> = self
            .sources
            .iter()
            .map(|(m, e)| format!("{m}={e}"))
            .collect();
        machines.join(", ")
    }
}

/// The result of one reconciliation.
pub struct Outcome {
    pub report: reconcile::Report,
    /// The planned file changes; already written unless this was a dry run.
    pub changes: Vec<FileChange>,
}

/// Scans every source concurrently, then plans and applies the changes under
/// the configuration lock.
pub async fn reconcile_once(settings: &Settings, dry_run: bool) -> Result<Outcome> {
    let scans: Vec<_> = settings
        .sources
        .iter()
        .map(|(machine, endpoint)| {
            let endpoint = endpoint.clone();
            let allowed_secrets = settings
                .allowed_secrets
                .get(machine)
                .cloned()
                .unwrap_or_default();
            let machine = machine.clone();
            let own = settings.own_container.clone();
            tokio::spawn(async move {
                let scan = docker::containers(&endpoint)
                    .await
                    .map(|c| labels::discover(&c, own.as_deref()))
                    .map_err(|e| format!("{endpoint}: {e:#}"));
                reconcile::Source {
                    machine,
                    scan,
                    allowed_secrets,
                }
            })
        })
        .collect();
    let mut sources = Vec::new();
    for scan in scans {
        sources.push(scan.await?);
    }
    let root = settings.config_dir.clone();
    let remove_after = settings.remove_after;
    let now = chrono::Utc::now();
    tokio::task::spawn_blocking(move || {
        let mut planned = None;
        config::edit_files(&root, dry_run, |loaded, files, preview| {
            let plan = reconcile::plan(loaded, files, preview, &sources, now, remove_after);
            planned = Some((plan.report, plan.changes.clone()));
            Ok((plan.changes, plan.config))
        })?;
        let (report, changes) = planned.expect("edit_files calls the planner");
        Ok(Outcome { report, changes })
    })
    .await?
}

/// One reconciliation from the command line.
pub async fn main(args: &[String]) -> Result<()> {
    let mut dry_run = false;
    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => bail!("unknown argument `{arg}`\n{USAGE}"),
        }
    }
    let settings = Settings::from_env()?.context(
        "set VESPER_DISCOVER_DOCKER to machine=endpoint pairs, such as docker1=unix:///var/run/docker.sock",
    )?;
    let outcome = reconcile_once(&settings, dry_run).await?;
    let r = &outcome.report;
    for line in &r.changes {
        println!("{}{line}", if dry_run { "would: " } else { "" });
    }
    if dry_run {
        for change in &outcome.changes {
            if let FileChange::Write { path, content } = change {
                println!("--- {path}\n{content}");
            }
        }
    }
    for line in &r.warnings {
        println!("warning: {line}");
    }
    for line in &r.errors {
        eprintln!("error: {line}");
    }
    if r.changes.is_empty() && r.errors.is_empty() {
        println!("discovered services are up to date");
    }
    if !r.errors.is_empty() {
        bail!("discovery finished with {} error(s)", r.errors.len());
    }
    Ok(())
}

/// Reconciles every interval, forever. Warnings and errors repeat on every
/// scan; each is logged only when it first appears, and one line says when
/// all of them have cleared.
pub async fn run(settings: Settings) {
    tracing::info!(
        "Docker label discovery every {}s for {}",
        settings.interval.as_secs(),
        settings.describe()
    );
    let mut shown = BTreeSet::new();
    let mut tick = tokio::time::interval(settings.interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let mut now = BTreeSet::new();
        match reconcile_once(&settings, false).await {
            Ok(outcome) => {
                for line in &outcome.report.changes {
                    tracing::info!("discovery: {line}");
                }
                for line in outcome.report.warnings {
                    if !shown.contains(&line) {
                        tracing::warn!("discovery: {line}");
                    }
                    now.insert(line);
                }
                for line in outcome.report.errors {
                    if !shown.contains(&line) {
                        tracing::error!("discovery: {line}");
                    }
                    now.insert(line);
                }
            }
            Err(e) => {
                let line = format!("{e:#}");
                if !shown.contains(&line) {
                    tracing::error!("discovery: {line}");
                }
                now.insert(line);
            }
        }
        if now.is_empty() && !shown.is_empty() {
            tracing::info!("discovery: all problems have cleared");
        }
        shown = now;
    }
}
