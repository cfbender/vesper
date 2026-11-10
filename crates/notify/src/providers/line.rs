use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("line", "LINE Messaging API", FIELDS, build)
    .docs("https://developers.line.biz/en/reference/messaging-api/#send-push-message")
    .identity(&["user_id"]);

const FIELDS: &[Field] = &[
    Field::secret(
        "channel_access_token",
        "Long-lived channel access token from the Messaging API tab of the LINE Developers Console.",
    ),
    Field::text(
        "user_id",
        "Your user ID from the Basic settings tab, or a group or room ID the bot is in.",
    )
    .example("U4af4980629e8f0b6d1c4b0c1a2b3c4d5"),
];

/// Longest text message the API accepts, in characters.
const MAX_TEXT: usize = 5000;

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "to": s.str("user_id")?,
        "messages": [{ "type": "text", "text": truncate(&m.text(), MAX_TEXT) }],
    });
    Ok(vec![HttpRequest::post(
        "https://api.line.me/v2/bot/message/push",
    )
    .bearer(s.str("channel_access_token")?)
    .json(body)
    .into()])
}
