use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("max", "MAX", FIELDS, build)
    .docs("https://dev.max.ru/docs-api/methods/POST/messages")
    .identity(&["api_url", "chat_id"]);

const FIELDS: &[Field] = &[
    Field::secret(
        "bot_token",
        "Bot token from the MAX business platform (Chat bots → Settings).",
    ),
    Field::url("api_url", "Bot API server.").default("https://platform-api.max.ru"),
    Field::text(
        "chat_id",
        "Numeric ID of the chat or channel the bot posts to.",
    )
    .example("-71234567890123"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let chat_id = s.str("chat_id")?;
    if chat_id.parse::<i64>().is_err() {
        return Err("setting chat_id must be a numeric chat ID".into());
    }
    let url = join(&s.base_url("api_url")?, "messages");
    let request = HttpRequest::post(url)
        .query([("chat_id", chat_id)])?
        .header("Authorization", s.str("bot_token")?)
        .json(json!({ "text": m.text() }));
    Ok(vec![request.into()])
}
