use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("squadcast", "Squadcast", FIELDS, build)
    .docs("https://developers.squadcast.com/incoming-webhooks/payload-format")
    .identity(&["webhook_url"])
    .per_incident();

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "Incident Webhook URL of the Squadcast service.",
)
.example("https://api.squadcast.com/v2/incidents/api/0123456789abcdef0123456789abcdef")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = match m.status {
        Status::Up => json!({
            "status": "resolve",
            "event_id": m.dedup_key(),
        }),
        _ => json!({
            "message": m.title,
            "description": m.body(),
            "tags": { "source": "vesper" },
            "status": "trigger",
            "event_id": m.dedup_key(),
        }),
    };
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
