use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("discord", "Discord", FIELDS, build)
    .docs("https://discord.com/developers/docs/resources/webhook#execute-webhook")
    .identity(&["webhook_url", "thread_id"]);

const FIELDS: &[Field] = &[
    Field::secret_url(
        "webhook_url",
        "Webhook URL from Channel settings → Integrations → Webhooks.",
    )
    .example("https://discord.com/api/webhooks/123/abc"),
    Field::text("username", "Display name of the webhook.").default("Vesper"),
    Field::url("avatar_url", "Avatar image URL.").optional(),
    Field::text("thread_id", "Post into this existing thread or forum post.")
        .optional()
        .example("1234567890"),
    Field::text(
        "thread_name",
        "Create a forum post with this name (forum channels only).",
    )
    .optional(),
    Field::bool(
        "suppress_notifications",
        "Post without push and desktop notifications.",
    ),
];

const SUPPRESS_NOTIFICATIONS_FLAG: u32 = 1 << 12;

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let color = match m.status {
        Status::Down => 0xff0000,
        Status::Up => 0x00cc66,
        Status::Test => 0x5865f2,
    };
    let mut body = json!({
        "username": s.str("username")?,
        "embeds": [{
            "title": truncate(&m.title, 256),
            "description": truncate(&m.summary(), 4096),
            "url": m.link,
            "color": color,
            "timestamp": m.at.to_rfc3339(),
        }],
        "allowed_mentions": { "parse": [] },
    });
    if let Some(avatar) = s.get("avatar_url") {
        body["avatar_url"] = json!(avatar);
    }
    if let Some(name) = s.get("thread_name") {
        body["thread_name"] = json!(name);
    }
    if s.bool("suppress_notifications") {
        body["flags"] = json!(SUPPRESS_NOTIFICATIONS_FLAG);
    }
    let mut request = HttpRequest::post(s.str("webhook_url")?).json(body);
    if let Some(thread) = s.get("thread_id") {
        request = request.query([("thread_id", thread)])?;
    }
    Ok(vec![request.into()])
}
