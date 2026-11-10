use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("alertnow", "AlertNow", FIELDS, build)
    .docs("https://service.opsnow.com/docs/alertnow/en/user-guide-alertnow-en.html#standard")
    .identity(&["webhook_url"])
    .per_incident();

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "Webhook URL of an AlertNow Standard integration.",
)
.example("https://alertnowitgr.opsnow.com/integration/standard/v1/0123456789abcdef")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let (status, event_type) = match m.status {
        Status::Up => ("close", "INFO"),
        _ => ("open", "ERROR"),
    };
    let body = json!({
        "summary": format!("{} - {}", m.title, m.lines.join("; ")),
        "event_id": m.dedup_key(),
        "status": status,
        "event_type": event_type,
        "event_time": m.at.to_rfc3339(),
        "custom_details": { "link": m.link },
    });
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
