use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("onechat", "OneChat", FIELDS, build)
    .docs("https://chat-develop.one.th/docs")
    .identity(&["receiver_id", "bot_id"]);

const FIELDS: &[Field] = &[
    Field::secret(
        "access_token",
        "Bot access token from the OneChat developer console.",
    ),
    Field::text(
        "receiver_id",
        "OneChat user ID or group ID that receives the message.",
    )
    .example("U1234567890abcdef"),
    Field::text("bot_id", "ID of the bot that sends the message.").example("B1234567890abcdef"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "to": s.str("receiver_id")?,
        "bot_id": s.str("bot_id")?,
        "type": "text",
        "message": m.text(),
    });
    let request = HttpRequest::post("https://chat-api.one.th/message/api/v1/push_message")
        .bearer(s.str("access_token")?)
        .json(body)
        .accept(classify);
    Ok(vec![request.into()])
}

/// Responses carry `status`: `success`, or `fail` with a `message`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted if r.json().get("status").and_then(Value::as_str) == Some("fail") => {
            Verdict::Rejected(r.summary())
        }
        other => other,
    }
}
