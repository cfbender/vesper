use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("stackfield", "Stackfield", FIELDS, build)
    .docs("https://www.stackfield.com/developer-api")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "Chat message webhook URL from Room settings → Integrations.",
)
.example("https://www.stackfield.com/apiwh/e5a1cfbd-970e-45a1-b81c-3e004f9bdab5")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    // Formatted as Uptime Kuma does: the title in `+…+` emphasis.
    let body = json!({ "Title": format!("+{}+\n{}", m.title, m.body()) });
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
