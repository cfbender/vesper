use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("signl4", "SIGNL4", FIELDS, build)
    .docs("https://docs.signl4.com/integrations/webhook/webhook.html")
    .identity(&["webhook_url"])
    .per_incident();

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "Team webhook URL, which contains the team secret.",
)
.example("https://connect.signl4.com/webhook/abcd1234")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let status = match m.status {
        Status::Up => "resolved",
        _ => "new",
    };
    let body = json!({
        "title": m.title,
        "message": m.summary(),
        "link": m.link,
        "X-S4-SourceSystem": "Vesper",
        "X-S4-ExternalID": m.dedup_key(),
        "X-S4-Status": status,
    });
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
