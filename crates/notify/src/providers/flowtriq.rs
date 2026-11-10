use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("flowtriq", "Flowtriq", FIELDS, build)
    .docs("https://flowtriq.com/docs?section=webhooks")
    .identity(&["webhook_url"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::secret_url("webhook_url", "Flowtriq webhook endpoint.")
        .example("https://app.flowtriq.com/api/webhooks/0123456789abcdef"),
    Field::secret("api_key", "API key sent as the X-API-Key header.").optional(),
];

/// Posts `{source, status, monitor, msg}`; `status` is `down`, `up` or
/// `info` for tests, and `monitor` the service name.
fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let status = match m.status {
        Status::Down => "down",
        Status::Up => "up",
        Status::Test => "info",
    };
    let body = json!({
        "source": "vesper",
        "status": status,
        "monitor": m.incident.as_ref().map_or("Vesper", |i| i.service_name.as_str()),
        "msg": m.text(),
    });
    let mut request = HttpRequest::post(s.str("webhook_url")?).json(body);
    if let Some(key) = s.get("api_key") {
        request = request.header("X-API-Key", key);
    }
    Ok(vec![request.into()])
}
