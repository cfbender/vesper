//! Catalog-wide checks. Every provider must build valid requests for every
//! status, both with all settings filled and with only the required ones.
//! Set `VESPER_NOTIFY_KINDS=a,b` to check only some kinds.

use crate::{FieldKind, IncidentRef, Message, Provider, Request, Setting, Settings, Status};
use chrono::TimeZone;
use std::collections::BTreeMap;

fn sample(provider: &Provider, key: &str) -> Setting {
    let field = provider.field(key).unwrap();
    let text = |t: &str| Setting::Text(t.to_string());
    if let Some(example) = field.example {
        return match field.kind {
            FieldKind::List => Setting::List(vec![example.to_string()]),
            _ => text(example),
        };
    }
    match field.kind {
        FieldKind::Text => text(&format!("sample-{key}")),
        FieldKind::Url => text(&format!("https://example.com/{key}")),
        FieldKind::Bool => text("true"),
        FieldKind::Int => text(field.default.unwrap_or("1")),
        FieldKind::List => {
            Setting::List(vec![format!("sample-{key}-1"), format!("sample-{key}-2")])
        }
        FieldKind::Choice(values) => text(field.default.unwrap_or(values[0])),
    }
}

pub(crate) fn message(status: Status, per_incident: bool) -> Message {
    let at = chrono::Utc
        .with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .unwrap();
    Message {
        title: match status {
            Status::Down => "Vesper: 1 new problem(s)".into(),
            Status::Up => "Vesper: 1 resolved".into(),
            Status::Test => "Vesper test message".into(),
        },
        lines: vec!["DOWN Web (jelly/web) since 2026-11-10 11:58 UTC (2m), failing: UI".into()],
        link: "https://vesper.example.com/incidents/7".into(),
        status,
        incident: (per_incident && status != Status::Test).then(|| IncidentRef {
            id: 7,
            service_id: "jelly/web".into(),
            service_name: "Web".into(),
        }),
        at,
    }
}

fn check_url(kind: &str, url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| format!("{kind}: invalid URL {url}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(format!("{kind}: URL must be absolute http(s): {url}"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!("{kind}: URL must not carry userinfo: {url}"));
    }
    Ok(())
}

fn check_requests(kind: &str, requests: &[Request]) -> Result<(), String> {
    for request in requests {
        match request {
            Request::Http(http) => {
                check_url(kind, &http.url)?;
                if let Some(token) = &http.token {
                    check_url(kind, &token.request.url)?;
                }
            }
            Request::Mail(mail) => {
                if mail.to.is_empty() && mail.cc.is_empty() && mail.bcc.is_empty() {
                    return Err(format!("{kind}: email without recipients"));
                }
            }
        }
    }
    Ok(())
}

fn check_provider(p: &'static Provider) -> Result<(), String> {
    let kind = p.kind;
    let id_like = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    if !id_like(kind) || p.name.is_empty() || !p.docs.starts_with("https://") {
        return Err(format!(
            "{kind}: needs a snake_case kind, a name and https docs"
        ));
    }
    let mut keys = std::collections::BTreeSet::new();
    for f in p.fields {
        if !id_like(f.key) || f.key == "kind" || !keys.insert(f.key) || f.help.is_empty() {
            return Err(format!(
                "{kind}.{}: bad, reserved or duplicate key, or no help",
                f.key
            ));
        }
        if let (FieldKind::Choice(values), Some(d)) = (f.kind, f.default) {
            if !values.contains(&d) {
                return Err(format!("{kind}.{}: default is not a choice", f.key));
            }
        }
        if f.kind == FieldKind::Int && f.default.is_some_and(|d| d.parse::<i64>().is_err()) {
            return Err(format!("{kind}.{}: default is not an integer", f.key));
        }
        if f.secret && f.default.is_some() {
            return Err(format!("{kind}.{}: secrets cannot have defaults", f.key));
        }
    }
    if let Some(k) = p.identity.iter().find(|k| p.field(k).is_none()) {
        return Err(format!("{kind}: identity names undeclared setting {k}"));
    }
    let all: BTreeMap<String, Setting> = p
        .fields
        .iter()
        .map(|f| (f.key.to_string(), sample(p, f.key)))
        .collect();
    let required: BTreeMap<String, Setting> = p
        .fields
        .iter()
        .filter(|f| f.required)
        .map(|f| (f.key.to_string(), sample(p, f.key)))
        .collect();
    // Each choice value once, on top of all other settings.
    let mut variants = vec![("all settings".to_string(), all.clone())];
    variants.push(("required settings".into(), required));
    for f in p.fields {
        if let FieldKind::Choice(values) = f.kind {
            for v in values {
                let mut settings = all.clone();
                settings.insert(f.key.into(), Setting::Text(v.to_string()));
                variants.push((format!("{}={v}", f.key), settings));
            }
        }
    }
    for (label, values) in variants {
        let settings = Settings::new(p, values);
        for status in [Status::Down, Status::Up, Status::Test] {
            let built =
                std::panic::catch_unwind(|| (p.build)(&settings, &message(status, p.per_incident)))
                    .map_err(|_| format!("{kind} ({label}, {status:?}): build panicked"))?;
            let requests = built.map_err(|e| format!("{kind} ({label}, {status:?}): {e}"))?;
            if requests.is_empty() && status != Status::Up {
                return Err(format!("{kind} ({label}, {status:?}): built no request"));
            }
            check_requests(kind, &requests)?;
        }
    }
    Ok(())
}

#[test]
fn every_provider_builds_valid_requests() {
    let only: Option<Vec<String>> = std::env::var("VESPER_NOTIFY_KINDS")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
    let errors: Vec<String> = super::ALL
        .iter()
        .filter(|p| only.as_ref().is_none_or(|o| o.iter().any(|k| k == p.kind)))
        .filter_map(|p| check_provider(p).err())
        .collect();
    assert!(
        errors.is_empty(),
        "{} provider(s) failed:\n{}",
        errors.len(),
        errors.join("\n")
    );
}

#[test]
fn kinds_are_sorted_and_unique() {
    let kinds: Vec<&str> = super::ALL.iter().map(|p| p.kind).collect();
    let mut sorted = kinds.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(kinds, sorted);
}

/// The generated part of `docs/notifications.md`, one section per kind.
fn catalog_docs() -> String {
    let mut out = String::new();
    let mut providers: Vec<&Provider> = super::ALL.to_vec();
    providers.sort_by_key(|p| p.name.to_lowercase());
    for p in providers {
        out.push_str(&format!(
            "\n<a id=\"{}\"></a>\n### {} (`{}`)\n\n[API documentation]({}){}\n\n",
            p.kind.replace('_', "-"),
            p.name,
            p.kind,
            p.docs,
            if p.per_incident {
                " · per incident"
            } else {
                ""
            }
        ));
        out.push_str("| Setting | Value | Description |\n| --- | --- | --- |\n");
        for f in p.fields {
            let value = match f.kind {
                FieldKind::Text => "text".to_string(),
                FieldKind::Url => "URL".into(),
                FieldKind::Bool => "true/false".into(),
                FieldKind::Int => "integer".into(),
                FieldKind::List => "list".into(),
                FieldKind::Choice(values) => values
                    .iter()
                    .map(|v| format!("`{v}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            let mut notes = vec![];
            if f.secret {
                notes.push("*secret*".to_string());
            }
            if f.required {
                notes.push("*required*".into());
            }
            if let Some(d) = f.default {
                notes.push(format!("default `{d}`"));
            }
            let value = if notes.is_empty() {
                value
            } else {
                format!("{value}; {}", notes.join(", "))
            };
            let help = f.help.replace('|', "\\|").replace('\n', " ");
            out.push_str(&format!("| `{}` | {value} | {help} |\n", f.key));
        }
    }
    out
}

#[test]
fn docs_list_every_kind() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/notifications.md");
    let doc = std::fs::read_to_string(path).unwrap();
    let (start, end) = ("<!-- catalog:start -->\n", "<!-- catalog:end -->");
    let head = &doc[..doc.find(start).unwrap() + start.len()];
    let tail = &doc[doc.find(end).unwrap()..];
    let expected = format!("{head}{}{tail}", catalog_docs());
    if std::env::var_os("VESPER_UPDATE_DOCS").is_some() {
        std::fs::write(path, &expected).unwrap();
    } else {
        assert!(
            doc == expected,
            "docs/notifications.md is out of date; run VESPER_UPDATE_DOCS=1 cargo test -p vesper-notify"
        );
    }
}
