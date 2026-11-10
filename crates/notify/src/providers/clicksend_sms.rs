use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("clicksend_sms", "ClickSend SMS", FIELDS, build)
    .docs("https://developers.clicksend.com/docs/messaging/sms/other/send-sms")
    .identity(&["to_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text(
        "username",
        "API username of the ClickSend account or subaccount.",
    ),
    Field::secret("api_key", "API key of the account or subaccount."),
    Field::text("to_number", "Recipient phone number in E.164 format.").example("+61411111111"),
    Field::text(
        "sender_name",
        "Sender ID of up to 11 characters; leave empty to use a shared number.",
    )
    .optional()
    .example("Vesper"),
];

/// ClickSend only takes plain ASCII here, as in Uptime Kuma.
fn ascii(text: &str) -> String {
    text.chars().filter(char::is_ascii).collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut message = json!({
        "body": ascii(&m.text()),
        "to": s.str("to_number")?,
        "source": "vesper",
    });
    if let Some(sender) = s.get("sender_name") {
        message["from"] = json!(sender);
    }
    Ok(vec![HttpRequest::post(
        "https://rest.clicksend.com/v3/sms/send",
    )
    .basic(s.str("username")?, s.str("api_key")?)
    .header("Accept", "application/json")
    .json(json!({ "messages": [message] }))
    .accept(classify)
    .into()])
}

/// The request succeeds as a whole while each message carries its own
/// status; `THROTTLED` means the same text was just sent to the recipient.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    match r
        .json()
        .pointer("/data/messages/0/status")
        .and_then(Value::as_str)
    {
        Some("SUCCESS") => Verdict::Accepted,
        Some("THROTTLED") => Verdict::RetryAfter(30),
        Some(status) => Verdict::Rejected(format!("message status {status}")),
        None => Verdict::Transient(r.summary()),
    }
}
