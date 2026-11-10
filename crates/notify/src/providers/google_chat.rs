use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("google_chat", "Google Chat", FIELDS, build)
    .docs("https://developers.google.com/workspace/chat/quickstart/webhooks")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] = &[Field::secret_url(
    "webhook_url",
    "Webhook URL from the space's Apps & integrations → Webhooks.",
)
.example("https://chat.googleapis.com/v1/spaces/AAAA/messages?key=abc&token=def")];

/// Escapes the HTML that card text paragraphs interpret.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "fallbackText": m.title,
        "cardsV2": [{
            "cardId": "vesper",
            "card": {
                "header": { "title": m.title },
                "sections": [{
                    "widgets": [
                        { "textParagraph": { "text": escape(&m.summary()) } },
                        {
                            "buttonList": {
                                "buttons": [{
                                    "text": "Open in Vesper",
                                    "onClick": { "openLink": { "url": m.link } },
                                }],
                            },
                        },
                    ],
                }],
            },
        }],
    });
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
