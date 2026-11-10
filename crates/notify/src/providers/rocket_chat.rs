use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("rocket_chat", "Rocket.Chat", FIELDS, build)
    .docs("https://docs.rocket.chat/docs/integrations")
    .identity(&["webhook_url", "channel"]);

const FIELDS: &[Field] = &[
    Field::secret_url(
        "webhook_url",
        "Incoming webhook URL from Administration → Integrations.",
    )
    .example("https://chat.example.com/hooks/abc123/def456"),
    Field::text("username", "Display name of the post.").optional(),
    Field::text(
        "icon_emoji",
        "Emoji used as the post icon, such as `:rotating_light:`.",
    )
    .optional(),
    Field::text(
        "channel",
        "Channel (`#name`) or user (`@name`) to post to instead of the webhook's default.",
    )
    .optional()
    .example("#alerts"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = match m.status {
        Status::Test => json!({ "text": m.text() }),
        status => json!({
            "text": m.title,
            "attachments": [{
                "title": m.title,
                "title_link": m.link,
                "text": m.summary(),
                "color": if status == Status::Down { "#ff0000" } else { "#32cd32" },
            }],
        }),
    };
    for key in ["username", "icon_emoji", "channel"] {
        if let Some(value) = s.get(key) {
            body[key] = json!(value);
        }
    }
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
