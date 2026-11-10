use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("sms_planet", "SMSPLANET", FIELDS, build)
    .docs("https://smsplanet.pl/doc/slate/index.html")
    .identity(&["phone_numbers"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret("api_token", "API token from the SMSPLANET customer panel."),
    Field::list("phone_numbers", "Recipient phone numbers.").example("+48123456789"),
    Field::text(
        "sender_name",
        "Sender name of up to 11 characters, approved in the SMSPLANET panel.",
    )
    .example("Vesper"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    Ok(vec![HttpRequest::post("https://api2.smsplanet.pl/sms")
        .bearer(s.str("api_token")?)
        .form([
            ("from", s.str("sender_name")?.to_string()),
            ("to", s.required_list("phone_numbers")?.join(",")),
            ("msg", m.text()),
        ])
        .accept(classify)
        .into()])
}

/// Success is `{"messageId":"…"}`; errors carry `errorMsg` and `errorCode`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let json = r.json();
    if json.get("messageId").is_some_and(|id| !id.is_null()) {
        return Verdict::Accepted;
    }
    match json.get("errorMsg").and_then(Value::as_str) {
        Some(error) => Verdict::Rejected(error.to_string()),
        None => Verdict::Transient(r.summary()),
    }
}
