use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("halopsa", "HaloPSA", FIELDS, build)
    .docs("https://halopsa.com/guides/")
    .identity(&["webhook_url"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::secret_url("webhook_url", "Incoming webhook URL of the Halo instance.")
        .example("https://example.halopsa.com/api/v1/webhook"),
    Field::text(
        "username",
        "Username for Basic authentication of the webhook.",
    )
    .optional(),
    Field::secret("password", "Password for the username.").optional(),
];

/// Posts `{title, status, monitor, monitor_id, message, link, timestamp}`;
/// `status` is `DOWN`, `UP` or `TEST`, and `monitor_id` is the service ID.
fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "title": m.title,
        "status": m.status_word(),
        "monitor": m.incident.as_ref().map_or("Vesper", |i| i.service_name.as_str()),
        "monitor_id": m.incident.as_ref().map(|i| i.service_id.as_str()),
        "message": m.summary(),
        "link": m.link,
        "timestamp": m.at.to_rfc3339(),
    });
    let mut request = HttpRequest::post(s.str("webhook_url")?).json(body);
    if let (Some(user), Some(password)) = (s.get("username"), s.get("password")) {
        request = request.basic(user, password);
    }
    Ok(vec![request.into()])
}
