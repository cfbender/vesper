use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("clickup", "ClickUp", FIELDS, build)
    .docs("https://developer.clickup.com/reference/createchatmessage")
    .identity(&["workspace_id", "channel_id"]);

const FIELDS: &[Field] = &[
    Field::secret("token", "Personal API token (`pk_…`)."),
    Field::text(
        "workspace_id",
        "Workspace ID, the number after app.clickup.com/ in its URL.",
    )
    .example("9012345678"),
    Field::text("channel_id", "Chat channel ID, from the channel's URL.")
        .example("6-901234567890-8"),
    Field::bool("disable_url", "Leave the dashboard link out of messages."),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let text = if s.bool("disable_url") {
        m.summary()
    } else {
        m.body()
    };
    let body = json!({
        "type": "message",
        "content": format!("**{}**\n{text}", m.title),
        "content_format": "text/md",
    });
    let url = format!(
        "https://api.clickup.com/api/v3/workspaces/{}/chat/channels/{}/messages",
        encode(s.str("workspace_id")?),
        encode(s.str("channel_id")?)
    );
    Ok(vec![HttpRequest::post(url)
        .header("Authorization", s.str("token")?)
        .json(body)
        .into()])
}
