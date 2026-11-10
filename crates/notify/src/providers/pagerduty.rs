use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pagerduty", "PagerDuty", FIELDS, build)
    .docs("https://developer.pagerduty.com/docs/events-api-v2/trigger-events/")
    .identity(&["integration_key"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::secret(
        "integration_key",
        "Events API v2 integration (routing) key of the service.",
    ),
    Field::url("integration_url", "Events API endpoint.")
        .default("https://events.pagerduty.com/v2/enqueue"),
    Field::choice(
        "severity",
        &["info", "warning", "error", "critical"],
        "Severity of triggered alerts.",
    )
    .default("warning"),
    Field::choice(
        "auto_resolve",
        &["resolve", "acknowledge", "none"],
        "What a recovery does to the alert.",
    )
    .default("resolve"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let action = match (m.status, s.str("auto_resolve")?) {
        (Status::Up, "none") => return Ok(vec![]),
        (Status::Up, action) => action,
        _ => "trigger",
    };
    let source = m
        .incident
        .as_ref()
        .map_or("Vesper".to_string(), |i| i.service_id.clone());
    let summary = truncate(&format!("{} {}", m.title, m.summary()), 1024);
    let body = json!({
        "routing_key": s.str("integration_key")?,
        "event_action": action,
        "dedup_key": m.dedup_key(),
        "payload": {
            "summary": summary,
            "severity": s.str("severity")?,
            "source": source,
        },
        "client": "Vesper",
        "client_url": m.link,
    });
    Ok(vec![HttpRequest::post(s.str("integration_url")?)
        .json(body)
        .into()])
}
