use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("sevenio", "seven", FIELDS, build)
    .docs("https://docs.seven.io/en/rest-api/endpoints/sms")
    .identity(&["receiver"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret(
        "api_key",
        "API key from the seven dashboard (Developer section).",
    ),
    Field::text(
        "sender",
        "Sender: up to 11 alphanumeric or 16 numeric characters.",
    )
    .default("Vesper"),
    Field::text(
        "receiver",
        "Recipient number in international format; separate several with commas.",
    )
    .example("49176123456789"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "to": s.str("receiver")?,
        "from": s.str("sender")?,
        "text": m.text(),
    });
    Ok(vec![HttpRequest::post("https://gateway.seven.io/api/sms")
        .header("X-Api-Key", s.str("api_key")?)
        .header("Accept", "application/json")
        .json(body)
        .accept(classify)
        .into()])
}

/// The gateway answers 200 with a return code in `success`: 100 is
/// accepted, 600 (carrier) and 700 (unknown) are server-side failures, the
/// rest are request or account errors.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let json = r.json();
    let code = match json.get("success") {
        Some(Value::String(c)) => c.clone(),
        Some(v @ Value::Number(_)) => v.to_string(),
        _ => String::from_utf8_lossy(&r.body).trim().to_string(),
    };
    match code.as_str() {
        "100" => Verdict::Accepted,
        "600" | "700" => Verdict::Transient(format!("return code {code}")),
        _ => Verdict::Rejected(format!("return code {code}")),
    }
}
