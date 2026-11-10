use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("slack", "Slack", FIELDS, build)
    .docs("https://api.slack.com/messaging/webhooks")
    .identity(&["webhook_url", "channel"]);

const FIELDS: &[Field] = &[
    Field::secret_url(
        "webhook_url",
        "Incoming webhook URL from the Slack app's Incoming Webhooks page.",
    )
    .example("https://hooks.slack.com/services/T000/B000/XXXX"),
    Field::text(
        "username",
        "Display name of the post; only legacy webhooks honour it.",
    )
    .optional(),
    Field::text(
        "icon_emoji",
        "Emoji used as the post icon, such as `:rotating_light:`; only legacy webhooks honour it.",
    )
    .optional(),
    Field::text(
        "channel",
        "Channel to post into, such as `#alerts`; only legacy webhooks honour it.",
    )
    .optional(),
    Field::bool(
        "rich_message",
        "Send problems and recoveries as a coloured block message with a button.",
    ),
    Field::bool("channel_notify", "Mention @channel in every message."),
];

/// Escapes the characters Slack's mrkdwn treats as control sequences.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mention = if s.bool("channel_notify") {
        " <!channel>"
    } else {
        ""
    };
    let mut body = if s.bool("rich_message") && m.status != Status::Test {
        let color = if m.status == Status::Up {
            "#2eb886"
        } else {
            "#e01e5a"
        };
        json!({
            "text": format!("{}{mention}", escape(&m.title)),
            "attachments": [{
                "color": color,
                "blocks": [
                    {
                        "type": "header",
                        "text": { "type": "plain_text", "text": truncate(&m.title, 150) },
                    },
                    {
                        "type": "section",
                        "text": { "type": "mrkdwn", "text": truncate(&escape(&m.summary()), 3000) },
                    },
                    {
                        "type": "actions",
                        "elements": [{
                            "type": "button",
                            "text": { "type": "plain_text", "text": "Open in Vesper" },
                            "url": m.link,
                        }],
                    },
                ],
            }],
        })
    } else {
        json!({ "text": format!("{}{mention}", escape(&m.text())) })
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
