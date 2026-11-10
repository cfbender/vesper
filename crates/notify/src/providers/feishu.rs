use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("feishu", "Feishu (Lark)", FIELDS, build)
    .docs("https://open.feishu.cn/document/client-docs/bot-v3/add-custom-bot")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] = &[
    Field::secret_url("webhook_url", "Webhook URL of the group's custom bot.")
        .example("https://open.feishu.cn/open-apis/bot/v2/hook/abc123"),
    Field::secret(
        "secret",
        "Signing secret, when the bot has signature verification enabled.",
    )
    .optional(),
];

/// Base64 HMAC-SHA256 of an empty message keyed with `timestamp\nsecret`.
fn sign(timestamp: i64, secret: &str) -> String {
    base64(hmac_sha256(
        format!("{timestamp}\n{secret}").as_bytes(),
        b"",
    ))
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let template = match m.status {
        Status::Down => "red",
        Status::Up => "green",
        Status::Test => "blue",
    };
    let mut body = json!({
        "msg_type": "interactive",
        "card": {
            "config": { "wide_screen_mode": true },
            "header": {
                "title": { "tag": "plain_text", "content": m.title },
                "template": template,
            },
            "elements": [
                { "tag": "div", "text": { "tag": "plain_text", "content": m.summary() } },
                { "tag": "action", "actions": [{
                    "tag": "button",
                    "text": { "tag": "plain_text", "content": "Open in Vesper" },
                    "type": "primary",
                    "url": m.link,
                }] },
            ],
        },
    });
    if let Some(secret) = s.get("secret") {
        let timestamp = m.at.timestamp();
        body["timestamp"] = json!(timestamp.to_string());
        body["sign"] = json!(sign(timestamp, secret));
    }
    let request = HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .accept(classify);
    Ok(vec![request.into()])
}

/// The bot API reports `code` (or `StatusCode` in older responses) and `msg`.
fn classify(r: &Response) -> Verdict {
    let json = r.json();
    let code = json
        .get("code")
        .or_else(|| json.get("StatusCode"))
        .and_then(Value::as_i64);
    match (code, by_status(r)) {
        (Some(11232), _) => Verdict::RetryAfter(r.retry_after.unwrap_or(60).max(1)),
        (Some(0), Verdict::Accepted) => Verdict::Accepted,
        (_, Verdict::Accepted) => Verdict::Rejected(r.summary()),
        (_, other) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_like_the_documented_algorithm() {
        // Inputs from Feishu's sample code; result computed independently
        // with Python's hmac and base64 modules.
        assert_eq!(
            sign(1_599_360_473, "demo"),
            "l1N0gAcBjdwBvGm1xMjOF0XSyaLRpR7tuO5dHfhAYc8="
        );
    }
}
