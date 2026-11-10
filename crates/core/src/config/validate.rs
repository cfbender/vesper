//! Semantic validation of an assembled configuration.

use super::assemble::is_valid_id;
use super::yaml::safe_key;
use crate::model::{Check, CheckDefaults, CheckKind, Config, SecretRef};
use crate::secrets::{is_valid_env_name, is_valid_file_name};
use anyhow::{bail, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

/// Hop-by-hop and framing headers the probe controls itself.
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "connection",
    "content-length",
    "transfer-encoding",
    "upgrade",
    "te",
];
const MAX_DESCRIPTION: usize = 4000;
const MAX_INTERVAL: u64 = 7 * 24 * 3600;

pub fn validate(config: &Config) -> Result<()> {
    let mut errors = Vec::new();
    check_defaults("defaults", &config.defaults, &mut errors);
    check_network(config, &mut errors);
    if let Some(t) = &config.telegram {
        secret("telegram.bot_token", &t.bot_token, &mut errors);
        secret("telegram.chat_id", &t.chat_id, &mut errors);
        match url::Url::parse(&t.dashboard_url) {
            Ok(u) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => {
                if !u.username().is_empty() || u.password().is_some() {
                    errors.push("telegram.dashboard_url: must not contain credentials".into());
                }
            }
            _ => errors.push("telegram.dashboard_url: must be an absolute http(s) URL".into()),
        }
        if t.reminder_seconds.is_some_and(|s| s < 60) {
            errors.push("telegram.reminder_seconds: must be at least 60".into());
        }
    }
    no_case_collisions("machines", config.machines.keys(), &mut errors);
    let all_services: BTreeSet<String> = config
        .machines
        .iter()
        .flat_map(|(m, machine)| machine.services.keys().map(move |s| format!("{m}/{s}")))
        .collect();
    for (mid, machine) in &config.machines {
        let mp = format!("machines.{}", safe_key(mid));
        id(&mp, mid, &mut errors);
        text(&format!("{mp}.name"), &machine.name, &mut errors);
        text(&format!("{mp}.site"), &machine.site, &mut errors);
        long_text(
            &format!("{mp}.description"),
            &machine.description,
            &mut errors,
        );
        if let Some(h) = &machine.host_service {
            if !machine.services.contains_key(h) {
                errors.push(format!(
                    "{mp}.host_service: no service `{}` on this machine",
                    shown(h)
                ));
            }
        }
        no_case_collisions(
            &format!("{mp}.services"),
            machine.services.keys(),
            &mut errors,
        );
        for (sid, service) in &machine.services {
            let sp = format!("{mp}.services.{}", safe_key(sid));
            id(&sp, sid, &mut errors);
            text(&format!("{sp}.name"), &service.name, &mut errors);
            long_text(
                &format!("{sp}.description"),
                &service.description,
                &mut errors,
            );
            if service.icon.is_empty()
                || service.icon.len() > 200
                || !service
                    .icon
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
            {
                errors.push(format!(
                    "{sp}.icon: must be a catalog ID or upload:<filename>"
                ));
            }
            let me = format!("{mid}/{sid}");
            let mut seen = BTreeSet::new();
            for dep in &service.depends_on {
                if dep == &me {
                    errors.push(format!(
                        "{sp}.depends_on: a service cannot depend on itself"
                    ));
                } else if !all_services.contains(dep) {
                    errors.push(format!(
                        "{sp}.depends_on: unknown service `{}` (use machine/service)",
                        shown(dep)
                    ));
                } else if !seen.insert(dep) {
                    errors.push(format!("{sp}.depends_on: `{dep}` is listed twice"));
                }
            }
            for (i, w) in service.maintenance.iter().enumerate() {
                let wp = format!("{sp}.maintenance[{i}]");
                let start = chrono::DateTime::parse_from_rfc3339(&w.start);
                let end = chrono::DateTime::parse_from_rfc3339(&w.end);
                match (start, end) {
                    (Ok(s), Ok(e)) => {
                        if e <= s {
                            errors.push(format!("{wp}: end must be after start"));
                        } else if w.repeat_weekly && e - s > chrono::Duration::days(7) {
                            errors.push(format!("{wp}: a weekly window cannot exceed 7 days"));
                        }
                    }
                    (s, e) => {
                        if s.is_err() {
                            errors.push(format!("{wp}.start: must be an RFC3339 timestamp"));
                        }
                        if e.is_err() {
                            errors.push(format!("{wp}.end: must be an RFC3339 timestamp"));
                        }
                    }
                }
            }
            no_case_collisions(&format!("{sp}.checks"), service.checks.keys(), &mut errors);
            for (cid, check) in &service.checks {
                let cp = format!("{sp}.checks.{}", safe_key(cid));
                id(&cp, cid, &mut errors);
                check_check(&cp, check, &config.defaults, &mut errors);
            }
        }
    }
    dependency_cycles(config, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        errors.truncate(50);
        bail!("invalid configuration:\n- {}", errors.join("\n- "))
    }
}

fn shown(s: &str) -> &str {
    if s.len() <= 161
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/'))
    {
        s
    } else {
        "<invalid>"
    }
}

fn id(path: &str, id: &str, errors: &mut Vec<String>) {
    if !is_valid_id(id) {
        errors.push(format!(
            "{}: IDs use ASCII letters, digits, '-' and '_' (at most {} bytes, not `_index`)",
            path.rsplit_once('.').map_or(path, |(p, _)| p),
            super::MAX_ID_BYTES
        ));
    }
}

fn text(path: &str, value: &str, errors: &mut Vec<String>) {
    if value.len() > 200 || value.chars().any(char::is_control) {
        errors.push(format!(
            "{path}: at most 200 characters without control characters"
        ));
    }
}

fn long_text(path: &str, value: &str, errors: &mut Vec<String>) {
    if value.len() > MAX_DESCRIPTION
        || value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        errors.push(format!(
            "{path}: at most 4000 characters without control characters"
        ));
    }
}

fn no_case_collisions<'a>(
    path: &str,
    keys: impl Iterator<Item = &'a String>,
    errors: &mut Vec<String>,
) {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for k in keys {
        if let Some(prev) = seen.insert(k.to_ascii_lowercase(), k) {
            errors.push(format!(
                "{path}: `{}` and `{}` differ only by case",
                shown(prev),
                shown(k)
            ));
        }
    }
}

fn secret(path: &str, s: &SecretRef, errors: &mut Vec<String>) {
    match s {
        SecretRef::Env(n) if !is_valid_env_name(n) => errors.push(format!(
            "{path}: env reference must name a variable like NAME_1"
        )),
        SecretRef::File(n) if !is_valid_file_name(n) => errors.push(format!(
            "{path}: file reference must be a plain file name inside the secrets directory"
        )),
        _ => {}
    }
}

fn check_defaults(path: &str, d: &CheckDefaults, errors: &mut Vec<String>) {
    if !(1..=MAX_INTERVAL).contains(&d.interval_seconds) {
        errors.push(format!(
            "{path}: interval_seconds must be between 1 and {MAX_INTERVAL}"
        ));
    }
    if !(1..=300).contains(&d.timeout_seconds) {
        errors.push(format!("{path}: timeout_seconds must be between 1 and 300"));
    } else if d.timeout_seconds > d.interval_seconds {
        errors.push(format!(
            "{path}: timeout_seconds must not exceed interval_seconds"
        ));
    }
    if !(1..=100).contains(&d.failure_threshold) {
        errors.push(format!(
            "{path}: failure_threshold must be between 1 and 100"
        ));
    }
    if !(1..=100).contains(&d.recovery_threshold) {
        errors.push(format!(
            "{path}: recovery_threshold must be between 1 and 100"
        ));
    }
    if d.stale_after_seconds < d.interval_seconds {
        errors.push(format!(
            "{path}: stale_after_seconds must be at least interval_seconds"
        ));
    }
}

fn check_network(config: &Config, errors: &mut Vec<String>) {
    let n = &config.network;
    for c in &n.allowed_cidrs {
        if parse_cidr(c).is_none() {
            errors.push(format!(
                "network.allowed_cidrs: `{}` is not a CIDR such as 192.168.1.0/24",
                shown_cidr(c)
            ));
        }
    }
    if !(1..=256).contains(&n.max_concurrent) {
        errors.push("network.max_concurrent: must be between 1 and 256".into());
    }
    if !(1..=64 * 1024 * 1024).contains(&n.max_response_bytes) {
        errors.push("network.max_response_bytes: must be between 1 and 67108864".into());
    }
    if let Some(ca) = &n.private_ca_file {
        if !is_valid_file_name(ca) {
            errors.push(
                "network.private_ca_file: must be a plain file name inside the secrets directory"
                    .into(),
            );
        }
    }
}

fn shown_cidr(c: &str) -> &str {
    if c.len() <= 64
        && c.bytes()
            .all(|b| b.is_ascii_hexdigit() || matches!(b, b'.' | b':' | b'/'))
    {
        c
    } else {
        "<invalid>"
    }
}

/// Parses `addr/prefix` with the address masked to the prefix.
pub fn parse_cidr(s: &str) -> Option<(IpAddr, u8)> {
    let (addr, prefix) = s.split_once('/')?;
    let addr: IpAddr = addr.parse().ok()?;
    if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_digit()) || prefix.len() > 3 {
        return None;
    }
    let prefix: u8 = prefix.parse().ok()?;
    let max = if addr.is_ipv4() { 32 } else { 128 };
    (prefix <= max).then_some((addr, prefix))
}

fn check_check(path: &str, c: &Check, defaults: &CheckDefaults, errors: &mut Vec<String>) {
    text(&format!("{path}.name"), &c.name, errors);
    let policy = c.policy(defaults);
    let mut policy_errors = Vec::new();
    check_defaults(path, &policy, &mut policy_errors);
    errors.extend(
        policy_errors
            .into_iter()
            .map(|e| format!("{e} (after applying defaults)")),
    );
    let http_only =
        c.url.is_some() || c.body_contains.is_some() || !c.headers.is_empty() || c.follow_redirects;
    match c.kind {
        CheckKind::Http => {
            match c.url.as_deref().map(url::Url::parse) {
                None => errors.push(format!("{path}.url: required for http checks")),
                Some(Err(_)) => errors.push(format!("{path}.url: not a valid URL")),
                Some(Ok(u)) => {
                    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
                        errors.push(format!("{path}.url: must be an absolute http(s) URL"));
                    }
                    if !u.username().is_empty() || u.password().is_some() {
                        errors.push(format!("{path}.url: credentials in URLs are not allowed; use headers with secret references"));
                    }
                }
            }
            if !(100..=599).contains(&c.expected_status) {
                errors.push(format!(
                    "{path}.expected_status: must be between 100 and 599"
                ));
            }
            if c.body_contains
                .as_ref()
                .is_some_and(|b| b.is_empty() || b.len() > 1024)
            {
                errors.push(format!("{path}.body_contains: must be 1 to 1024 bytes"));
            }
            for (name, value) in &c.headers {
                let token = !name.is_empty()
                    && name.len() <= 128
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_|~`".contains(&b));
                if !token {
                    errors.push(format!(
                        "{path}.headers: invalid header name `{}`",
                        super::yaml::safe_key(name)
                    ));
                } else if FORBIDDEN_HEADERS
                    .iter()
                    .any(|h| name.eq_ignore_ascii_case(h))
                {
                    errors.push(format!("{path}.headers: `{name}` cannot be set"));
                }
                secret(
                    &format!("{path}.headers.{}", super::yaml::safe_key(name)),
                    value,
                    errors,
                );
            }
            if c.host.is_some() || c.port.is_some() {
                errors.push(format!(
                    "{path}: host and port apply to tcp checks; http checks use url"
                ));
            }
            if c.token.is_some() {
                errors.push(format!("{path}.token: applies to push checks only"));
            }
        }
        CheckKind::Tcp => {
            match c.host.as_deref() {
                None | Some("") => errors.push(format!("{path}.host: required for tcp checks")),
                Some(h) => {
                    let ok = h.len() <= 253
                        && (h.parse::<IpAddr>().is_ok()
                            || h.split('.').all(|l| {
                                !l.is_empty()
                                    && l.len() <= 63
                                    && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                                    && !l.starts_with('-')
                                    && !l.ends_with('-')
                            }));
                    if !ok {
                        errors.push(format!("{path}.host: must be a hostname or IP address"));
                    }
                }
            }
            if matches!(c.port, None | Some(0)) {
                errors.push(format!("{path}.port: required for tcp checks (1-65535)"));
            }
            if http_only {
                errors.push(format!(
                    "{path}: url, body_contains, headers and follow_redirects apply to http checks"
                ));
            }
            if c.token.is_some() {
                errors.push(format!("{path}.token: applies to push checks only"));
            }
        }
        CheckKind::Push => {
            match &c.token {
                None => errors.push(format!(
                    "{path}.token: push checks need a secret reference for the heartbeat token"
                )),
                Some(t) => secret(&format!("{path}.token"), t, errors),
            }
            if !(1..=MAX_INTERVAL).contains(&c.expected_interval_seconds) {
                errors.push(format!(
                    "{path}.expected_interval_seconds: must be between 1 and {MAX_INTERVAL}"
                ));
            }
            if c.grace_seconds > MAX_INTERVAL {
                errors.push(format!(
                    "{path}.grace_seconds: must be at most {MAX_INTERVAL}"
                ));
            }
            if http_only || c.host.is_some() || c.port.is_some() {
                errors.push(format!(
                    "{path}: push checks receive heartbeats; remove url/host/port/headers"
                ));
            }
        }
    }
}

fn dependency_cycles(config: &Config, errors: &mut Vec<String>) {
    let graph: BTreeMap<String, &Vec<String>> = config
        .machines
        .iter()
        .flat_map(|(m, machine)| {
            machine
                .services
                .iter()
                .map(move |(s, svc)| (format!("{m}/{s}"), &svc.depends_on))
        })
        .collect();
    // 0 = unvisited, 1 = on stack, 2 = done.
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    for start in graph.keys() {
        if state.get(start.as_str()).copied().unwrap_or(0) != 0 {
            continue;
        }
        let mut stack: Vec<(&str, usize)> = vec![(start, 0)];
        state.insert(start, 1);
        while let Some((node, i)) = stack.pop() {
            let deps = graph.get(node).map_or(&[][..], |d| d.as_slice());
            if i < deps.len() {
                stack.push((node, i + 1));
                let next = deps[i].as_str();
                match state.get(next).copied().unwrap_or(0) {
                    0 if graph.contains_key(next) => {
                        state.insert(next, 1);
                        stack.push((next, 0));
                    }
                    1 => {
                        errors.push(format!(
                            "depends_on: dependency cycle through `{}`",
                            shown(next)
                        ));
                        return;
                    }
                    _ => {}
                }
            } else {
                state.insert(node, 2);
            }
        }
    }
}
