use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("keep", "Keep", FIELDS, build)
    .docs("https://docs.keephq.dev/overview/alertseverityandstatus")
    .identity(&["server_url"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::url("server_url", "Keep API server.").example("https://api.keephq.dev"),
    Field::secret("api_key", "Keep API key with permission to write alerts."),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let (status, severity) = match m.status {
        Status::Down => ("firing", "critical"),
        Status::Up => ("resolved", "critical"),
        Status::Test => ("firing", "info"),
    };
    let mut body = json!({
        "name": m.title,
        "status": status,
        "severity": severity,
        "lastReceived": m.at.to_rfc3339(),
        "message": m.summary(),
        "description": m.body(),
        "source": ["vesper"],
        "fingerprint": m.dedup_key(),
        "url": m.link,
    });
    if let Some(i) = &m.incident {
        body["service"] = json!(i.service_name);
    }
    let url = join(&s.base_url("server_url")?, "alerts/event");
    Ok(vec![HttpRequest::post(url)
        .header("X-API-KEY", s.str("api_key")?)
        .json(body)
        .into()])
}
