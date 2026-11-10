use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("smspartner", "SMS Partner", FIELDS, build)
    .docs("https://www.docpartner.dev/en/api/sms-partner")
    .identity(&["phone_numbers"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key from the SMS Partner dashboard."),
    Field::list(
        "phone_numbers",
        "Recipient phone numbers in international format.",
    )
    .example("+33612345678"),
    Field::text(
        "sender_name",
        "Sender name of 3 to 11 letters and digits; longer names are cut.",
    )
    .example("Vesper"),
];

/// SMS Partner does not take non-ASCII characters here, as in Uptime Kuma.
fn ascii(text: &str) -> String {
    text.chars().filter(char::is_ascii).collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let sender: String = s.str("sender_name")?.chars().take(11).collect();
    let message: String = ascii(&m.text()).chars().take(639).collect();
    Ok(vec![HttpRequest::post("https://api.smspartner.fr/v1/send")
        .header("Accept", "application/json")
        .header("cache-control", "no-cache")
        .json(json!({
            "apiKey": s.str("api_key")?,
            "phoneNumbers": s.required_list("phone_numbers")?.join(","),
            "sender": sender,
            "message": message,
        }))
        .accept(classify)
        .into()])
}

/// The API answers `{"success":true,…}` or `{"success":false,"code":…,"message":…}`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    if r.json().get("success").and_then(Value::as_bool) == Some(true) {
        Verdict::Accepted
    } else {
        Verdict::Rejected(r.summary())
    }
}
