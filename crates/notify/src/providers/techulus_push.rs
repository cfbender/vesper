use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("techulus_push", "Push by Techulus", FIELDS, build)
    .docs("https://docs.push.techulus.com/reference/notify")
    .identity(&["api_key", "channel"]);

const SOUNDS: &[&str] = &[
    "default", "arcade", "correct", "fail", "harp", "reveal", "bubble", "doorbell", "flute",
    "money", "scifi", "clear", "elevator", "guitar", "pop",
];

const FIELDS: &[Field] = &[
    Field::secret(
        "api_key",
        "Account or team API key; it decides which devices are notified.",
    ),
    Field::text(
        "channel",
        "Notification channel: letters, digits and hyphens.",
    )
    .optional()
    .example("vesper"),
    Field::choice("sound", SOUNDS, "Notification sound.").optional(),
    Field::bool(
        "time_sensitive",
        "Deliver problems immediately, even in Do Not Disturb (iOS only).",
    )
    .default("true"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = json!({
        "title": m.title,
        "body": m.summary(),
        "link": m.link,
        "timeSensitive": m.status == Status::Down && s.bool("time_sensitive"),
    });
    if let Some(channel) = s.get("channel") {
        if !channel
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("setting channel may contain only letters, digits and hyphens".into());
        }
        body["channel"] = json!(channel);
    }
    if let Some(sound) = s.get("sound") {
        body["sound"] = json!(sound);
    }
    Ok(vec![HttpRequest::post(
        "https://push.techulus.com/api/v1/notify",
    )
    .header("x-api-key", s.str("api_key")?)
    .json(body)
    .accept(|r| body_code(r, "/success", "true"))
    .into()])
}
