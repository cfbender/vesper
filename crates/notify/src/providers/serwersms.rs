use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("serwersms", "SerwerSMS.pl", FIELDS, build)
    .docs("https://dev.serwersms.pl/en/https-api-v2/sending-sms-messages-with-the-same-content")
    .identity(&["recipient_type", "recipient"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text(
        "username",
        "API user name from Interface settings → HTTPS XML API → Users.",
    ),
    Field::secret("password", "Password of the API user."),
    Field::choice(
        "recipient_type",
        &["phone", "group"],
        "Send to one phone number or to a contact group.",
    )
    .default("phone"),
    Field::text(
        "recipient",
        "Phone number, or the group ID from the Customer Panel when recipient_type is group.",
    )
    .example("+48500600700"),
    Field::text(
        "sender_name",
        "Approved sender name for SMS FULL; without one an SMS ECO is sent.",
    )
    .optional()
    .example("Vesper"),
];

/// Uptime Kuma sends ASCII only, which keeps each SMS at 160 characters.
fn ascii(text: &str) -> String {
    text.chars().filter(char::is_ascii).collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = json!({
        "username": s.str("username")?,
        "password": s.str("password")?,
        "text": ascii(&m.text()),
    });
    let recipient = s.str("recipient")?;
    match s.str("recipient_type")? {
        "group" => body["group_id"] = json!(recipient),
        _ => body["phone"] = json!(recipient),
    }
    if let Some(sender) = s.get("sender_name") {
        body["sender"] = json!(sender);
    }
    Ok(vec![HttpRequest::post(
        "https://api2.serwersms.pl/messages/send_sms",
    )
    .json(body)
    .accept(classify)
    .into()])
}

/// Failures carry `{"success":false,"error":{"code":…,"type":…,"message":…}}`.
fn classify(r: &Response) -> Verdict {
    let json = r.json();
    if r.success() && json.get("success").and_then(Value::as_bool) == Some(true) {
        return Verdict::Accepted;
    }
    match (by_status(r), json.get("error")) {
        (Verdict::Accepted, Some(error)) => Verdict::Rejected(format!(
            "error {}: {}",
            error.get("code").map_or_else(String::new, Value::to_string),
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
        )),
        (Verdict::Accepted, None) => Verdict::Transient(r.summary()),
        (other, _) => other,
    }
}
