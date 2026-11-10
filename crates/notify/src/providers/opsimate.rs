use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("opsimate", "OpsiMate", FIELDS, build)
    .docs("https://docs.opsimate.dev/docs/integrations/overview")
    .identity(&["server_url"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::url("server_url", "OpsiMate server.").example("https://opsimate.example.com:3001"),
    Field::secret(
        "api_token",
        "API token of the OpsiMate server (its `API_TOKEN`).",
    ),
];

/// Uses the generic custom alert endpoint: an alert with status `resolved`
/// resolves the alert with the same ID.
fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let (status, severity) = match m.status {
        Status::Down => ("firing", "critical"),
        Status::Up => ("resolved", "critical"),
        Status::Test => ("firing", "info"),
    };
    let mut tags = json!({ "source": "vesper" });
    if let Some(i) = &m.incident {
        tags["service"] = json!(i.service_id);
    }
    let body = json!({
        "id": m.dedup_key(),
        "status": status,
        "alertName": m.title,
        "summary": m.summary(),
        "alertUrl": m.link,
        "severity": severity,
        "tags": tags,
    });
    let url = join(&s.base_url("server_url")?, "api/v1/alerts/custom");
    Ok(vec![HttpRequest::post(url)
        .header("x-api-token", s.str("api_token")?)
        .json(body)
        .into()])
}
