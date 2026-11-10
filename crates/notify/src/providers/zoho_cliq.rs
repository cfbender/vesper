use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("zoho_cliq", "Zoho Cliq", FIELDS, build)
    .docs("https://www.zoho.com/cliq/help/restapi/v2/#Post_Message_Channel")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "Channel message URL including its `zapikey` webhook token.",
)
.example("https://cliq.zoho.com/api/v2/channelsbyname/alerts/message?zapikey=abc123")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let heading = match m.status {
        Status::Down => format!("🔴 {}", m.title),
        Status::Up => format!("### ✅ {}", m.title),
        Status::Test => m.title.clone(),
    };
    let body = json!({ "text": format!("{heading}\n{}", m.body()) });
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
