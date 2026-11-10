//! Docker container labels to Vesper services.
//!
//! A label `vesper.<service>.<path>` sets the value at `<path>` inside the
//! service's YAML, with dots between keys: `vesper.web.name`,
//! `vesper.web.checks.health.url`, `vesper.web.checks.health.headers.Authorization.file`.
//! Labels from every container on one machine are merged by service ID; the
//! same label with different values on two containers is a conflict.
//!
//! Labels under [`CONFIG_SECTIONS`] set Vesper's own configuration instead,
//! such as `vesper.network.allowed_cidrs`. They are read only from the
//! container Vesper itself runs in.

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use vesper_core::config::{is_valid_id, sanitize_serde};
use vesper_core::{Config, Service};
use vesper_notify::FieldKind;

pub const PREFIX: &str = "vesper.";

/// First label segments that set configuration rather than a service.
pub const CONFIG_SECTIONS: &[&str] = &["defaults", "network", "notifications", "machines"];

const BOOLS: &[&str] = &["paused", "notifications", "enabled", "follow_redirects"];
const NUMBERS: &[&str] = &[
    "port",
    "expected_status",
    "expected_interval_seconds",
    "grace_seconds",
    "interval_seconds",
    "timeout_seconds",
    "failure_threshold",
    "recovery_threshold",
    "stale_after_seconds",
];

#[derive(Debug, Clone, Default)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

/// What one machine's containers declare.
#[derive(Debug, Default, PartialEq)]
pub struct Discovered {
    /// Services whose labels form a well-typed service, by service ID.
    pub services: BTreeMap<String, Service>,
    /// Service IDs whose labels are invalid or conflicting, with the reason.
    /// Their current files are kept unchanged.
    pub broken: BTreeMap<String, String>,
    /// Problems that cannot be tied to one service ID, such as a malformed
    /// label key. While any exist, nothing on the machine is removed.
    pub problems: Vec<String>,
    /// Configuration labels on containers other than Vesper's, which are
    /// ignored.
    pub warnings: Vec<String>,
    /// When Vesper's own container is among these: the partial root mapping
    /// its configuration labels set, or why they are invalid.
    pub config: Option<Result<Value, String>>,
}

/// A label key that is safe to show in messages.
fn shown(key: &str) -> &str {
    if key.len() <= 200
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
    {
        key
    } else {
        "<label>"
    }
}

/// `own` is the ID, or an ID prefix, of the container Vesper runs in.
pub fn discover(containers: &[Container], own: Option<&str>) -> Discovered {
    let mut out = Discovered::default();
    // service ID -> path below the service -> (value, container)
    let mut declared: BTreeMap<&str, BTreeMap<&str, (&str, &str)>> = BTreeMap::new();
    let mut config: Option<BTreeMap<&str, &str>> = None;
    for container in containers {
        let name = shown(&container.name);
        let is_own = own.is_some_and(|id| !id.is_empty() && container.id.starts_with(id));
        if is_own {
            config.get_or_insert_default();
        }
        for (key, value) in &container.labels {
            let Some(rest) = key.strip_prefix(PREFIX) else {
                continue;
            };
            if rest
                .split_once('.')
                .is_some_and(|(first, _)| CONFIG_SECTIONS.contains(&first))
            {
                match config.as_mut().filter(|_| is_own) {
                    Some(config) => {
                        config.insert(rest, value);
                    }
                    None => out.warnings.push(format!(
                        "container {name}: label `{}` sets Vesper's own configuration, which is read only from the labels of the container Vesper runs in; ignored",
                        shown(key)
                    )),
                }
                continue;
            }
            let (service, path) = match rest.split_once('.') {
                Some((s, p)) if is_valid_id(s) && !p.is_empty() => (s, p),
                _ => {
                    out.problems.push(format!(
                        "container {name}: label `{}` is not vesper.<service>.<field> with a service ID of ASCII letters, digits, '-' and '_'",
                        shown(key)
                    ));
                    continue;
                }
            };
            let labels = declared.entry(service).or_default();
            match labels.get(path) {
                Some((other_value, other)) if *other_value != value.as_str() => {
                    out.broken.entry(service.to_string()).or_insert_with(|| {
                        format!(
                            "label `{}` differs between containers {other} and {name}",
                            shown(key)
                        )
                    });
                }
                Some(_) => {}
                None => {
                    labels.insert(path, (value, name));
                }
            }
        }
    }
    for (service, labels) in declared {
        if out.broken.contains_key(service) {
            continue;
        }
        match build(service, &labels) {
            Ok(s) => {
                out.services.insert(service.to_string(), s);
            }
            Err(e) => {
                out.broken.insert(service.to_string(), e);
            }
        }
    }
    out.config = config.map(|labels| build_config(&labels));
    out
}

/// Turns configuration labels (`network.allowed_cidrs` → value) into the
/// partial root mapping they set.
fn build_config(labels: &BTreeMap<&str, &str>) -> Result<Value, String> {
    let mut root = Map::new();
    for (path, value) in labels {
        let label = format!("{PREFIX}{path}");
        let segments: Vec<&str> = path.split('.').collect();
        let leaf =
            config_typed(&segments, value).map_err(|e| format!("label `{}` {e}", shown(&label)))?;
        insert(&mut root, &segments, leaf)
            .map_err(|()| format!("label `{}` conflicts with another label", shown(&label)))?;
    }
    // Labels are text; settings the channel's kind declares as true/false
    // become booleans.
    if let Some(Value::Object(channels)) = root
        .get_mut("notifications")
        .and_then(|n| n.get_mut("channels"))
    {
        for channel in channels.values_mut().filter_map(Value::as_object_mut) {
            let Some(provider) = channel
                .get("kind")
                .and_then(Value::as_str)
                .and_then(vesper_notify::provider)
            else {
                continue;
            };
            for (key, value) in channel.iter_mut() {
                if provider
                    .field(key)
                    .is_some_and(|f| f.kind == FieldKind::Bool)
                {
                    if let Ok(b) = boolean(value.as_str().unwrap_or_default()) {
                        *value = b;
                    }
                }
            }
        }
    }
    let value = Value::Object(root);
    serde_path_to_error::deserialize::<_, Config>(value.clone()).map_err(|e| {
        let path = e.path().to_string();
        let message = sanitize_serde(&e.into_inner().to_string());
        if path == "." {
            message
        } else {
            format!("label `{}`: {message}", shown(&format!("{PREFIX}{path}")))
        }
    })?;
    Ok(value)
}

fn config_typed(segments: &[&str], value: &str) -> Result<Value, String> {
    match segments {
        ["network", "allowed_cidrs"] => Ok(list(value)),
        ["network", "max_concurrent" | "max_response_bytes"]
        | ["defaults", _]
        | ["notifications", "reminder_seconds"] => number(value),
        ["machines", _, "services", ..] => Err(
            "cannot be set here; services come from vesper.<service>.* labels on their containers"
                .into(),
        ),
        _ => Ok(Value::String(value.into())),
    }
}

fn list(value: &str) -> Value {
    Value::Array(
        value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.into()))
            .collect(),
    )
}

fn number(value: &str) -> Result<Value, String> {
    value
        .parse::<u64>()
        .map(Value::from)
        .map_err(|_| "must be a whole number".to_string())
}

fn boolean(value: &str) -> Result<Value, String> {
    match value {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ => Err("must be true or false".into()),
    }
}

fn build(service: &str, labels: &BTreeMap<&str, (&str, &str)>) -> Result<Service, String> {
    let mut root = Map::new();
    for (path, (value, _)) in labels {
        let label = format!("{PREFIX}{service}.{path}");
        let segments: Vec<&str> = path.split('.').collect();
        let leaf = typed(&segments, value).map_err(|e| format!("label `{}` {e}", shown(&label)))?;
        insert(&mut root, &segments, leaf)
            .map_err(|()| format!("label `{}` conflicts with another label", shown(&label)))?;
    }
    if let Some(Value::Object(checks)) = root.get_mut("checks") {
        for check in checks.values_mut() {
            if let Value::Object(c) = check {
                if !c.contains_key("kind") {
                    let kind = if c.contains_key("token") {
                        "push"
                    } else if c.contains_key("host") || c.contains_key("port") {
                        "tcp"
                    } else {
                        "http"
                    };
                    c.insert("kind".into(), kind.into());
                }
            }
        }
    }
    serde_path_to_error::deserialize::<_, Service>(Value::Object(root)).map_err(|e| {
        let path = e.path().to_string();
        let message = sanitize_serde(&e.into_inner().to_string());
        if path == "." {
            message
        } else {
            format!(
                "label `{}`: {message}",
                shown(&format!("{PREFIX}{service}.{path}"))
            )
        }
    })
}

/// Converts a label's text into the JSON type its field needs.
fn typed(segments: &[&str], value: &str) -> Result<Value, String> {
    let field = match segments {
        [field] => Some(*field),
        ["checks", _, field] => Some(*field),
        _ => None,
    };
    Ok(match field {
        Some("maintenance") if segments.len() == 1 => {
            return Err(
                "cannot be set by a label; maintenance windows need hand-written YAML".into(),
            )
        }
        Some("depends_on") if segments.len() == 1 => list(value),
        Some(f) if BOOLS.contains(&f) => boolean(value)?,
        Some(f) if NUMBERS.contains(&f) => number(value)?,
        _ => Value::String(value.into()),
    })
}

fn insert(map: &mut Map<String, Value>, segments: &[&str], leaf: Value) -> Result<(), ()> {
    let (last, parents) = segments.split_last().ok_or(())?;
    let mut current = map;
    for segment in parents {
        let next = current
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        current = next.as_object_mut().ok_or(())?;
    }
    if current.contains_key(*last) {
        return Err(());
    }
    current.insert(last.to_string(), leaf);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vesper_core::{CheckKind, SecretRef};

    fn container(name: &str, labels: &[(&str, &str)]) -> Container {
        Container {
            id: name.into(),
            name: name.into(),
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn labels_become_typed_services_merged_across_containers() {
        let found = discover(
            &[
                container(
                    "app",
                    &[
                        ("vesper.paperless.name", "1984"),
                        ("vesper.paperless.notifications", "false"),
                        ("vesper.paperless.depends_on", "docker1/host, nas/storage"),
                        ("vesper.paperless.checks.api.url", "https://p.lan/api/"),
                        ("vesper.paperless.checks.api.expected_status", "401"),
                        (
                            "vesper.paperless.checks.api.headers.Authorization.file",
                            "paperless-token",
                        ),
                        ("com.docker.compose.service", "app"),
                    ],
                ),
                container(
                    "db",
                    &[
                        ("vesper.paperless.name", "1984"),
                        ("vesper.paperless.checks.db.host", "10.0.0.5"),
                        ("vesper.paperless.checks.db.port", "5432"),
                        ("vesper.paperless.checks.beat.token.env", "BEAT_TOKEN"),
                    ],
                ),
            ],
            None,
        );
        assert!(
            found.broken.is_empty() && found.problems.is_empty(),
            "{found:?}"
        );
        let s = &found.services["paperless"];
        assert_eq!(s.name, "1984", "text fields are never coerced");
        assert!(!s.notifications);
        assert_eq!(s.depends_on, ["docker1/host", "nas/storage"]);
        let api = &s.checks["api"];
        assert_eq!((api.kind, api.expected_status), (CheckKind::Http, 401));
        assert_eq!(
            api.headers["Authorization"],
            SecretRef::File("paperless-token".into())
        );
        let db = &s.checks["db"];
        assert_eq!((db.kind, db.port), (CheckKind::Tcp, Some(5432)));
        assert_eq!(s.checks["beat"].kind, CheckKind::Push);
    }

    #[test]
    fn bad_labels_are_attributed_without_echoing_values() {
        let found = discover(
            &[
                container(
                    "a",
                    &[
                        ("vesper.web.checks.h.url", "http://a/"),
                        ("vesper.literal.checks.h.url", "http://a/"),
                        (
                            "vesper.literal.checks.h.headers.Authorization",
                            "Bearer s3cret",
                        ),
                        ("vesper.typo.checks.h.prot", "80"),
                        ("vesper.num.checks.h.port", "eighty"),
                        ("vesper.bad id.name", "x"),
                    ],
                ),
                container("b", &[("vesper.web.checks.h.url", "http://b/")]),
            ],
            None,
        );
        assert!(found.services.is_empty());
        assert!(found.broken["web"].contains("differs between containers a and b"));
        assert!(found.broken["literal"].contains("headers.Authorization"));
        assert!(found.broken["typo"].contains("unknown field `prot`"));
        assert!(found.broken["num"].contains("must be a whole number"));
        assert_eq!(found.problems.len(), 1);
        assert!(found.problems[0].contains("container a: label `<label>`"));
        let all = format!("{found:?}");
        assert!(!all.contains("s3cret") && !all.contains("eighty"));
    }
}
