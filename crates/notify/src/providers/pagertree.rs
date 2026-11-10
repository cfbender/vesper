use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pagertree", "PagerTree", FIELDS, build)
    .docs("https://pagertree.com/docs/integration-guides/webhook")
    .identity(&["integration_url"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::secret_url(
        "integration_url",
        "Endpoint URL of a PagerTree webhook integration.",
    )
    .example("https://api.pagertree.com/integration/int_abc123"),
    Field::choice(
        "urgency",
        &["low", "medium", "high", "critical"],
        "Urgency of created alerts.",
    )
    .default("medium"),
    Field::choice(
        "auto_resolve",
        &["resolve", "none"],
        "What a recovery does to the alert.",
    )
    .default("resolve"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = match (m.status, s.str("auto_resolve")?) {
        (Status::Up, "none") => return Ok(vec![]),
        (Status::Up, _) => json!({
            "event_type": "resolve",
            "id": m.dedup_key(),
        }),
        _ => json!({
            "event_type": "create",
            "id": m.dedup_key(),
            "title": m.title,
            "description": m.body(),
            "urgency": s.str("urgency")?,
        }),
    };
    Ok(vec![HttpRequest::post(s.str("integration_url")?)
        .json(body)
        .into()])
}
