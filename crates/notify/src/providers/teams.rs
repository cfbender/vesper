use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("teams", "Microsoft Teams", FIELDS, build)
    .docs("https://learn.microsoft.com/en-us/microsoftteams/platform/webhooks-and-connectors/how-to/add-incoming-webhook")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "Workflows (Power Automate) or incoming webhook URL of the channel.",
)
.example("https://example.webhook.office.com/webhookb2/abc/IncomingWebhook/def/ghi")];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let style = match m.status {
        Status::Down => "attention",
        Status::Up => "good",
        Status::Test => "emphasis",
    };
    let mut card_body = vec![json!({
        "type": "Container",
        "style": style,
        "items": [{
            "type": "TextBlock",
            "text": m.title,
            "size": "Medium",
            "weight": "Bolder",
            "wrap": true,
        }],
    })];
    card_body.extend(m.lines.iter().map(|line| {
        json!({
            "type": "TextBlock",
            "text": line,
            "wrap": true,
        })
    }));
    let body = json!({
        "type": "message",
        "summary": m.title,
        "attachments": [{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "contentUrl": null,
            "content": {
                "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                "type": "AdaptiveCard",
                "version": "1.5",
                "body": card_body,
                "actions": [{
                    "type": "Action.OpenUrl",
                    "title": "Open in Vesper",
                    "url": m.link,
                }],
            },
        }],
    });
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
