use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("egosms", "EgoSMS", FIELDS, build)
    .docs("https://developers.pahappa.com/")
    .identity(&["phone_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("username", "EgoSMS account username."),
    Field::secret("password", "EgoSMS account password."),
    Field::text(
        "phone_number",
        "Recipient number in international format without a plus sign.",
    )
    .example("256700123456"),
    Field::text("sender", "Sender name or number of up to 11 characters.").default("EGOSMS"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let request = HttpRequest::get("https://www.egosms.co/api/v1/plain/").query([
        ("number", s.str("phone_number")?.to_string()),
        ("message", m.text()),
        ("username", s.str("username")?.to_string()),
        ("password", s.str("password")?.to_string()),
        ("sender", s.str("sender")?.to_string()),
        ("priority", "0".to_string()),
    ])?;
    Ok(vec![request.accept(classify).into()])
}

/// The plain API answers 200 with `OK` on success and an error text otherwise.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let body = String::from_utf8_lossy(&r.body);
    if body.trim().starts_with("OK") {
        Verdict::Accepted
    } else {
        Verdict::Rejected(r.summary())
    }
}
