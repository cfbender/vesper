use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("vkteams", "VK Teams", FIELDS, build)
    .docs("https://teams.vk.com/botapi/")
    .identity(&["base_url", "chat_id"]);

const FIELDS: &[Field] = &[
    Field::secret("bot_token", "Bot token from Metabot."),
    Field::text(
        "chat_id",
        "Chat ID or user login (email) the bot writes to; message the bot first.",
    )
    .example("alerts@chat.agent"),
    Field::url(
        "base_url",
        "Bot API server, for on-premise VK Teams installations.",
    )
    .default("https://myteam.mail.ru"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = join(&s.base_url("base_url")?, "bot/v1/messages/sendText");
    let request = HttpRequest::get(url)
        .query([
            ("token", s.str("bot_token")?),
            ("chatId", s.str("chat_id")?),
            ("text", &m.text()),
        ])?
        .accept(classify);
    Ok(vec![request.into()])
}

/// The Bot API answers 200 with `{"ok": false, "description": …}` on errors.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted if r.json().get("ok").and_then(Value::as_bool) == Some(true) => {
            Verdict::Accepted
        }
        Verdict::Accepted => Verdict::Rejected(r.summary()),
        other => other,
    }
}
