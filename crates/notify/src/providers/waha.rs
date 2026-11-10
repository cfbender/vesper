use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("waha", "WAHA (WhatsApp HTTP API)", FIELDS, build)
    .docs("https://waha.devlike.pro/docs/how-to/send-messages/")
    .identity(&["server_url", "session", "chat_id"]);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Address of the self-hosted WAHA server.")
        .example("http://waha.lan:3000"),
    Field::secret("api_key", "API key, when WAHA is started with WAHA_API_KEY.").optional(),
    Field::text("session", "WAHA session that sends the message.").default("default"),
    Field::text(
        "chat_id",
        "Chat ID: a phone number with country code (`11234567890@c.us`) or a group (`123456789012345678@g.us`).",
    )
    .example("11234567890@c.us"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = join(&s.base_url("server_url")?, "api/sendText");
    let mut request = HttpRequest::post(url)
        .header("Accept", "application/json")
        .json(json!({
            "session": s.str("session")?,
            "chatId": s.str("chat_id")?,
            "text": m.text(),
        }));
    if let Some(key) = s.get("api_key") {
        request = request.header("X-Api-Key", key);
    }
    Ok(vec![request.into()])
}
