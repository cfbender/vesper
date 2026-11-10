use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("openwa", "OpenWA", FIELDS, build)
    .docs("https://docs.open-wa.org/guides/sending-messages")
    .identity(&["server_url", "session", "chat_ids"])
    .max_chars(4096);

const FIELDS: &[Field] = &[
    Field::url(
        "server_url",
        "Address of the self-hosted OpenWA gateway, without `/api`.",
    )
    .example("http://openwa.lan:2785"),
    Field::secret("api_key", "OpenWA API key with operator role or higher."),
    Field::text("session", "ID of the session that sends the message.")
        .example("8f3c2b1a-9d4e-4c7a-8b2f-1e6d5a4c3b2a"),
    Field::list(
        "chat_ids",
        "Chat IDs such as `628123456789@c.us`, `120363021234567890@g.us` or `1234567890@lid`.",
    )
    .example("628123456789@c.us"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = join(
        &s.base_url("server_url")?,
        &format!(
            "api/sessions/{}/messages/send-text",
            encode(s.str("session")?)
        ),
    );
    let key = s.str("api_key")?;
    let text = m.text();
    Ok(s.required_list("chat_ids")?
        .into_iter()
        .map(|chat_id| {
            HttpRequest::post(url.clone())
                .header("X-Api-Key", key)
                .header("Accept", "application/json")
                .json(json!({ "chatId": chat_id, "text": text }))
                .into()
        })
        .collect())
}
