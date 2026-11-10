use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("telnyx", "Telnyx", FIELDS, build)
    .docs("https://developers.telnyx.com/api/messaging/send-message")
    .identity(&["to_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key from the Telnyx portal."),
    Field::text(
        "from_number",
        "Telnyx number in E.164 format, short code or alphanumeric sender ID to send from.",
    )
    .example("+15551234567"),
    Field::text("to_number", "Recipient phone number in E.164 format.").example("+15559876543"),
    Field::text(
        "messaging_profile_id",
        "Messaging profile to send through, required for alphanumeric sender IDs.",
    )
    .optional()
    .example("40017f8e-49bd-4f16-9e3d-ef103f916228"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = json!({
        "from": s.str("from_number")?,
        "to": s.str("to_number")?,
        "text": m.text(),
    });
    if let Some(profile) = s.get("messaging_profile_id") {
        body["messaging_profile_id"] = json!(profile);
    }
    Ok(vec![HttpRequest::post(
        "https://api.telnyx.com/v2/messages",
    )
    .bearer(s.str("api_key")?)
    .json(body)
    .into()])
}
