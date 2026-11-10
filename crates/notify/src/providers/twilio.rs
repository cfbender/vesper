use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("twilio", "Twilio", FIELDS, build)
    .docs("https://www.twilio.com/docs/messaging/api/message-resource#create-a-message-resource")
    .identity(&["to_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("account_sid", "Account SID from the Twilio console, starting with AC.")
        .example("ACxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"),
    Field::text(
        "api_key_sid",
        "API key SID (SK…) to authenticate with instead of the account SID; auth_token is then the key's secret.",
    )
    .optional()
    .example("SKxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"),
    Field::secret(
        "auth_token",
        "Auth token of the account, or the secret of the API key.",
    ),
    Field::text(
        "from_number",
        "Twilio number in E.164 format, or an alphanumeric sender ID, to send from.",
    )
    .example("+15551234567"),
    Field::text("to_number", "Recipient phone number in E.164 format.").example("+15557654321"),
    Field::text(
        "messaging_service_sid",
        "Messaging Service SID (MG…) to send through.",
    )
    .optional()
    .example("MG0123456789abcdef0123456789abcdef"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let account = s.str("account_sid")?;
    let user = s.get("api_key_sid").unwrap_or(account);
    let mut form = vec![
        ("To", s.str("to_number")?.to_string()),
        ("From", s.str("from_number")?.to_string()),
        ("Body", m.text()),
    ];
    if let Some(service) = s.get("messaging_service_sid") {
        form.push(("MessagingServiceSid", service.to_string()));
    }
    let url = format!(
        "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
        encode(account)
    );
    Ok(vec![HttpRequest::post(url)
        .basic(user, s.str("auth_token")?)
        .form(form)
        .into()])
}
