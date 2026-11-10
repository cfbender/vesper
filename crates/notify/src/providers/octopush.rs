use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("octopush", "Octopush", FIELDS, build)
    .docs("https://dev.octopush.com/en/sms-gateway-api-documentation/send-sms")
    .identity(&["phone_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key from the Octopush account."),
    Field::text(
        "login",
        "E-mail address of the Octopush account (api-login).",
    )
    .example("ops@example.com"),
    Field::text("phone_number", "Recipient phone number in E.164 format.").example("+33612345678"),
    Field::choice(
        "sms_type",
        &["sms_premium", "sms_low_cost"],
        "Premium or low-cost route.",
    )
    .default("sms_premium"),
    Field::text("sender_name", "Sender name of 3 to 11 letters and digits.")
        .optional()
        .example("Vesper"),
];

/// Octopush does not take non-ASCII characters here, as in Uptime Kuma.
fn ascii(text: &str) -> String {
    text.chars().filter(char::is_ascii).collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = json!({
        "recipients": [{ "phone_number": s.str("phone_number")? }],
        "text": ascii(&m.text()),
        "type": s.str("sms_type")?,
        "purpose": "alert",
    });
    if let Some(sender) = s.get("sender_name") {
        body["sender"] = json!(sender);
    }
    Ok(vec![HttpRequest::post(
        "https://api.octopush.com/v1/public/sms-campaign/send",
    )
    .header("api-key", s.str("api_key")?)
    .header("api-login", s.str("login")?)
    .header("cache-control", "no-cache")
    .json(body)
    .into()])
}
