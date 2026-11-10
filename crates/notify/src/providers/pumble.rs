use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pumble", "Pumble", FIELDS, build)
    .docs("https://pumble.com/help/integrations/add-pumble-apps/incoming-webhooks-for-pumble/")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] =
    &[
        Field::secret_url("webhook_url", "Incoming webhook URL of the channel.")
            .example("https://api.pumble.com/workspaces/abc/incomingWebhooks/postMessage/def"),
    ];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let color = match m.status {
        Status::Down => "#DC3645",
        Status::Up | Status::Test => "#5BDD8B",
    };
    let body = json!({
        "attachments": [{
            "title": m.title,
            "text": m.body(),
            "color": color,
        }],
    });
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
