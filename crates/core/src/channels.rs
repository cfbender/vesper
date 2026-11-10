//! Notification channel settings: validation against the provider catalog
//! and resolution of secret references for delivery.

use crate::model::{ChannelConfig, SecretRef};
use crate::secrets;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use vesper_notify::{FieldKind, Provider, Setting, Settings};

const MAX_SETTING_CHARS: usize = 2000;

fn secret_ref(value: &Value) -> Option<SecretRef> {
    value
        .is_object()
        .then(|| serde_json::from_value(value.clone()).ok())
        .flatten()
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Appends one error per invalid setting. Messages name settings, never values.
pub fn validate(path: &str, channel: &ChannelConfig, errors: &mut Vec<String>) {
    let Some(provider) = vesper_notify::provider(&channel.kind) else {
        let kind = if channel.kind.is_empty() {
            "is required".to_string()
        } else {
            "names no known channel kind (see docs/notifications.md)".to_string()
        };
        errors.push(format!("{path}.kind: {kind}"));
        return;
    };
    for key in channel.settings.keys() {
        if provider.field(key).is_none() {
            // Only setting-shaped keys are echoed; anything else may be a
            // value pasted in the wrong place.
            let plausible = key.len() <= 40
                && key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            let key = if plausible { key } else { "<invalid>" };
            errors.push(format!(
                "{path}.{key}: not a setting of kind {}",
                provider.kind
            ));
        }
    }
    for field in provider.fields {
        let fp = format!("{path}.{}", field.key);
        let Some(value) = channel.settings.get(field.key).filter(|v| !v.is_null()) else {
            if field.required {
                errors.push(format!("{fp}: required"));
            }
            continue;
        };
        if let Some(secret) = secret_ref(value) {
            if field.kind == FieldKind::Bool {
                errors.push(format!("{fp}: must be true or false"));
            }
            secret_reference(&fp, &secret, errors);
            continue;
        }
        if field.secret {
            errors.push(format!(
                "{fp}: credentials must be secret references such as {{file: name}} or {{env: NAME}}"
            ));
            continue;
        }
        if let Some(problem) = literal_problem(field.kind, value) {
            errors.push(format!("{fp}: {problem}"));
        }
    }
}

fn secret_reference(path: &str, s: &SecretRef, errors: &mut Vec<String>) {
    match s {
        SecretRef::Env(n) if !secrets::is_valid_env_name(n) => errors.push(format!(
            "{path}: env reference must name a variable like NAME_1"
        )),
        SecretRef::File(n) if !secrets::is_valid_file_name(n) => errors.push(format!(
            "{path}: file reference must be a plain file name inside the secrets directory"
        )),
        _ => {}
    }
}

fn literal_problem(kind: FieldKind, value: &Value) -> Option<String> {
    let bounded = |t: &str| {
        (t.chars().count() > MAX_SETTING_CHARS
            || t.chars().any(|c| c.is_control() && c != '\n' && c != '\t'))
        .then(|| format!("at most {MAX_SETTING_CHARS} characters without control characters"))
    };
    match kind {
        FieldKind::List => match value {
            Value::Array(items) => items
                .iter()
                .map(|i| match scalar_text(i) {
                    Some(t) if !i.is_boolean() => bounded(&t),
                    _ => Some("list items must be text".into()),
                })
                .find(Option::is_some)
                .flatten(),
            v => match scalar_text(v) {
                Some(t) => bounded(&t),
                None => Some("must be a list of text or one comma-separated text".into()),
            },
        },
        FieldKind::Bool => (!value.is_boolean()).then(|| "must be true or false".into()),
        FieldKind::Int => {
            let ok = value.as_i64().is_some()
                || value
                    .as_str()
                    .is_some_and(|s| s.trim().parse::<i64>().is_ok());
            (!ok).then(|| "must be an integer".into())
        }
        FieldKind::Choice(values) => match scalar_text(value) {
            Some(t) if values.contains(&t.as_str()) => None,
            _ => Some(format!("must be one of: {}", values.join(", "))),
        },
        FieldKind::Url => match value.as_str().map(url::Url::parse) {
            Some(Ok(u)) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => {
                (!u.username().is_empty() || u.password().is_some()).then(|| {
                    "must not contain credentials; use a secret reference for the whole URL".into()
                })
            }
            _ => Some("must be an absolute http(s) URL".into()),
        },
        FieldKind::Text => match value {
            Value::String(_) | Value::Number(_) => bounded(&scalar_text(value)?),
            _ => Some("must be text".into()),
        },
    }
}

/// Resolves a validated channel's settings, reading secret references.
/// Errors name the setting, never its value.
pub fn resolve(channel: &ChannelConfig, secrets_root: &Path) -> Result<Settings> {
    let provider: &'static Provider =
        vesper_notify::provider(&channel.kind).ok_or_else(|| anyhow!("unknown channel kind"))?;
    let mut values = BTreeMap::new();
    for (key, value) in &channel.settings {
        let Some(field) = provider.field(key) else {
            continue;
        };
        let setting = if let Some(secret) = secret_ref(value) {
            let resolved = secrets::resolve(&secret, secrets_root)
                .map_err(|_| anyhow!("could not resolve the {key} secret"))?;
            Setting::Text(resolved)
        } else if let Value::Array(items) = value {
            Setting::List(items.iter().filter_map(scalar_text).collect())
        } else if let Some(text) = scalar_text(value) {
            Setting::Text(text)
        } else {
            continue;
        };
        values.insert(field.key.to_string(), setting);
    }
    Ok(Settings::new(provider, values))
}
