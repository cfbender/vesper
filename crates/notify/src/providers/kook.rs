use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("kook", "KOOK", FIELDS, build)
    .docs("https://developer.kookapp.cn/doc/http/message")
    .identity(&["channel_id"]);

const FIELDS: &[Field] = &[
    Field::secret("bot_token", "Bot token from the KOOK developer center."),
    Field::text(
        "channel_id",
        "ID of the text channel the bot posts to (Uptime Kuma calls it Guild ID); copy it with developer mode on.",
    )
    .example("1234567890123456"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "type": 1,
        "target_id": s.str("channel_id")?,
        "content": m.text(),
    });
    Ok(vec![HttpRequest::post(
        "https://www.kookapp.cn/api/v3/message/create",
    )
    .header("Authorization", format!("Bot {}", s.str("bot_token")?))
    .json(body)
    .accept(|r| body_code(r, "/code", "0"))
    .into()])
}
