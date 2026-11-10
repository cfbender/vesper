use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("mattermost", "Mattermost", FIELDS, build)
    .docs("https://developers.mattermost.com/integrate/webhooks/incoming/")
    .identity(&["webhook_url", "channel"]);

const FIELDS: &[Field] = &[
    Field::secret_url(
        "webhook_url",
        "Incoming webhook URL from Integrations → Incoming Webhooks.",
    )
    .example("https://mattermost.example.com/hooks/xxxgeneratedkeyxxx"),
    Field::text("username", "Display name of the post.").default("Vesper"),
    Field::url("icon_url", "Image URL used as the post icon.").optional(),
    Field::text(
        "icon_emoji",
        "Emoji used as the post icon; two space-separated emojis, such as `:white_check_mark: :red_circle:`, set the recovery and problem icons.",
    )
    .optional(),
    Field::text(
        "channel",
        "Channel name to post into instead of the webhook's default channel.",
    )
    .optional()
    .example("town-square"),
];

/// The icon for this status: with two emojis the first is for recoveries
/// (and tests) and the second for problems, otherwise the one emoji.
fn icon_emoji(s: &Settings, status: Status) -> Option<&str> {
    let raw = s.get("icon_emoji")?;
    let parts: Vec<&str> = raw.split_whitespace().collect();
    match (parts.as_slice(), status) {
        ([_, problem, ..], Status::Down) => Some(problem),
        ([recovery, _, ..], _) => Some(recovery),
        _ => Some(raw),
    }
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let color = match m.status {
        Status::Down => "#FF0000",
        Status::Up => "#32CD32",
        Status::Test => "#439FE0",
    };
    let mut body = json!({
        "username": s.str("username")?,
        "attachments": [{
            "fallback": m.text(),
            "color": color,
            "title": m.title,
            "title_link": m.link,
            "text": m.summary(),
        }],
    });
    if let Some(channel) = s.get("channel") {
        body["channel"] = json!(channel.to_lowercase());
    }
    if let Some(emoji) = icon_emoji(s, m.status) {
        body["icon_emoji"] = json!(emoji);
    }
    if let Some(url) = s.get("icon_url") {
        body["icon_url"] = json!(url);
    }
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
