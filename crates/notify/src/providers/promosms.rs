use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("promosms", "PromoSMS", FIELDS, build)
    .docs("https://promosms.com/pomoc/api/api-informacje-ogolne")
    .identity(&["phone_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("login", "PromoSMS API login."),
    Field::secret("password", "PromoSMS API password."),
    Field::text("phone_number", "Recipient phone number with country code.").example("48500600700"),
    Field::choice(
        "sms_type",
        &["0", "1", "3", "4"],
        "Message type: 0 flash, 1 ECO, 3 FULL (with sender name) or 4 SPEED.",
    )
    .default("3"),
    Field::text(
        "sender_name",
        "Sender name of up to 11 characters, for FULL and SPEED messages.",
    )
    .optional()
    .example("Vesper"),
    Field::bool(
        "allow_long_sms",
        "Allow up to 4 concatenated messages (639 characters) instead of one (159).",
    ),
];

/// Special characters would shorten an SMS to 70 characters; Uptime Kuma
/// drops non-ASCII text.
fn ascii(text: &str) -> String {
    text.chars().filter(char::is_ascii).collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let long = s.bool("allow_long_sms");
    let text: String = ascii(&m.text())
        .chars()
        .take(if long { 639 } else { 159 })
        .collect();
    let kind: u8 = s
        .str("sms_type")?
        .parse()
        .map_err(|_| "setting sms_type must be a number")?;
    let mut body = json!({
        "recipients": [s.str("phone_number")?],
        "text": text,
        "long-sms": long,
        "type": kind,
    });
    if let Some(sender) = s.get("sender_name") {
        body["sender"] = json!(sender);
    }
    Ok(vec![HttpRequest::post(
        "https://promosms.com/api/rest/v3_2/sms",
    )
    .basic(s.str("login")?, s.str("password")?)
    .header("Accept", "application/json")
    .json(body)
    .accept(classify)
    .into()])
}

/// The API reports success as `response.status` 0.
fn classify(r: &Response) -> Verdict {
    body_code(r, "/response/status", "0")
}
