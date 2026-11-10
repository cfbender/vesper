//! Bounded active probes (HTTP(S) and TCP) under a file-controlled network policy.
//!
//! Every destination is resolved, each actual address is checked against
//! [`NetworkPolicy::allowed_cidrs`] and the always-denied address classes, and
//! the socket is connected to that validated address (no second resolution,
//! no ambient proxy). Messages are sanitized: they never contain response
//! bodies, secret values or full URLs.

mod http;
mod policy;

use policy::AddressPolicy;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use vesper_core::{Check, CheckDefaults, CheckKind, NetworkPolicy, ProbeResult, SecretRef};

/// Upper bound on any configured timeout, so a typo cannot park a probe for hours.
const MAX_TIMEOUT_SECONDS: u64 = 300;
const MAX_MESSAGE_CHARS: usize = 300;
const MAX_RESOLVED_ADDRESSES: usize = 16;

/// Runs one active check once. The whole probe, including DNS, TLS and body
/// reads, is bounded by the check's effective `timeout_seconds`.
pub async fn run(
    check: &Check,
    defaults: &CheckDefaults,
    network: &NetworkPolicy,
    secrets_root: &Path,
) -> ProbeResult {
    let started = Instant::now();
    let seconds = check
        .policy(defaults)
        .timeout_seconds
        .clamp(1, MAX_TIMEOUT_SECONDS);
    let outcome = match tokio::time::timeout(
        Duration::from_secs(seconds),
        execute(check, network, secrets_root),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_) => Err(Failure::new(format!("timed out after {seconds}s"))),
    };
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match outcome {
        Ok(message) => ProbeResult {
            healthy: true,
            latency_ms,
            message: sanitize(message),
        },
        Err(Failure(message)) => ProbeResult {
            healthy: false,
            latency_ms,
            message,
        },
    }
}

async fn execute(
    check: &Check,
    network: &NetworkPolicy,
    secrets_root: &Path,
) -> Result<String, Failure> {
    if check.kind == CheckKind::Push {
        return Err(Failure::new(
            "push checks are updated by heartbeats and cannot be actively probed",
        ));
    }
    let policy = AddressPolicy::new(&network.allowed_cidrs)?;
    match check.kind {
        CheckKind::Http => http::check(check, network, &policy, secrets_root).await,
        CheckKind::Tcp => tcp(check, &policy).await,
        CheckKind::Push => unreachable!("handled above"),
    }
}

async fn tcp(check: &Check, policy: &AddressPolicy) -> Result<String, Failure> {
    let host = check
        .host
        .as_deref()
        .ok_or_else(|| Failure::new("TCP check has no host"))?;
    let port = check
        .port
        .filter(|port| *port != 0)
        .ok_or_else(|| Failure::new("TCP check has no port"))?;
    let target = parse_host(host)?;
    let (_stream, peer) = connect(&target, port, policy).await?;
    Ok(format!("TCP connect to {peer} succeeded"))
}

/// A sanitized, bounded failure message. Construct only from text that is
/// safe to show: never response content, secret values or raw URLs.
#[derive(Debug)]
pub(crate) struct Failure(String);

impl Failure {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(sanitize(message.into()))
    }
}

fn sanitize(message: String) -> String {
    let cleaned: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if cleaned.chars().count() <= MAX_MESSAGE_CHARS {
        cleaned
    } else {
        let mut cut: String = cleaned.chars().take(MAX_MESSAGE_CHARS - 1).collect();
        cut.push('…');
        cut
    }
}

pub(crate) enum Target {
    Ip(IpAddr),
    Name(String),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Ip(ip) => write!(f, "{ip}"),
            Target::Name(name) => write!(f, "`{name}`"),
        }
    }
}

fn parse_host(raw: &str) -> Result<Target, Failure> {
    let raw = raw.trim();
    let unbracketed = raw
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(raw);
    if let Ok(ip) = unbracketed.parse::<IpAddr>() {
        return Ok(Target::Ip(ip));
    }
    let name = raw.strip_suffix('.').unwrap_or(raw);
    let valid = !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
    if valid {
        Ok(Target::Name(raw.to_ascii_lowercase()))
    } else {
        Err(Failure::new(format!(
            "host `{raw}` is not a valid hostname or IP address"
        )))
    }
}

/// Resolves the target, keeps only addresses the policy permits and connects
/// to one of them directly. Returns the validated address actually connected.
pub(crate) async fn connect(
    target: &Target,
    port: u16,
    policy: &AddressPolicy,
) -> Result<(TcpStream, SocketAddr), Failure> {
    let mut resolved: Vec<IpAddr> = match target {
        Target::Ip(ip) => vec![*ip],
        Target::Name(name) => tokio::net::lookup_host((name.as_str(), port))
            .await
            .map_err(|_| Failure::new(format!("could not resolve {target}")))?
            .map(|addr| addr.ip())
            .take(MAX_RESOLVED_ADDRESSES)
            .collect(),
    };
    dedup(&mut resolved);
    if resolved.is_empty() {
        return Err(Failure::new(format!("{target} resolved to no addresses")));
    }

    let mut allowed = Vec::new();
    let mut denied = Vec::new();
    for ip in resolved {
        match policy.permit(ip) {
            Ok(ip) => allowed.push(ip),
            Err(reason) => denied.push(reason),
        }
    }
    dedup(&mut allowed);
    if allowed.is_empty() {
        return Err(Failure::new(format!(
            "destination {target} denied by network policy: {}",
            denied.join(", ")
        )));
    }

    let mut last_error = String::new();
    for ip in allowed {
        let addr = SocketAddr::new(ip, port);
        match TcpStream::connect(addr).await {
            Ok(stream) => match stream.peer_addr() {
                Ok(peer) if peer.ip().to_canonical() == ip && peer.port() == port => {
                    return Ok((stream, addr))
                }
                _ => last_error = format!("{addr}: connected peer did not match"),
            },
            Err(error) => last_error = format!("{addr}: {}", error.kind()),
        }
    }
    Err(Failure::new(format!(
        "could not connect to {target}: {last_error}"
    )))
}

fn dedup(addresses: &mut Vec<IpAddr>) {
    let mut seen = Vec::with_capacity(addresses.len());
    addresses.retain(|ip| {
        let fresh = !seen.contains(ip);
        if fresh {
            seen.push(*ip);
        }
        fresh
    });
}

/// Resolves a configured secret reference. The value must never reach a
/// message; the error is core's outermost description, which names the
/// reference but never its content.
/// Checks one resolved address of a notification destination under
/// `allowed_cidrs`: public addresses are permitted, private networks only
/// when listed, and loopback, link-local and metadata addresses never. Returns
/// the canonical address, or the reason it is denied.
pub fn permit_notification_address(ip: IpAddr, allowed_cidrs: &[String]) -> Result<IpAddr, String> {
    let policy = AddressPolicy::new(allowed_cidrs).map_err(|Failure(m)| m)?;
    policy::permit_notification(&policy, ip)
}

pub(crate) fn resolve_secret(secret: &SecretRef, secrets_root: &Path) -> Result<String, String> {
    vesper_core::secrets::resolve(secret, secrets_root).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_future_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        let check = Check::default();
        let defaults = CheckDefaults::default();
        let network = NetworkPolicy::default();
        let future = run(&check, &defaults, &network, Path::new("/nonexistent"));
        assert_send(&future);
    }

    #[test]
    fn messages_are_single_line_and_bounded() {
        let long = format!("a\r\nb\u{1b}[31m{}", "x".repeat(1000));
        let clean = sanitize(long);
        assert!(!clean.chars().any(char::is_control));
        assert_eq!(clean.chars().count(), MAX_MESSAGE_CHARS);
        assert!(clean.starts_with("a  b [31m"));
    }

    #[test]
    fn hosts_are_validated() {
        assert!(matches!(parse_host("10.0.0.1"), Ok(Target::Ip(_))));
        assert!(matches!(parse_host("[fd00::1]"), Ok(Target::Ip(_))));
        assert!(matches!(parse_host("nas.lan"), Ok(Target::Name(_))));
        for bad in ["", "a b", "host/path", "user@host", "host:22", "a..b"] {
            assert!(parse_host(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
