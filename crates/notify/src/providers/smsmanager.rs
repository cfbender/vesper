use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("smsmanager", "SmsManager", FIELDS, build)
    .docs("https://smsmanager.cz/docs/api-reference/http-api/send")
    .identity(&["numbers"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key from app.smsmanager.com."),
    Field::list(
        "numbers",
        "Recipient phone numbers in international format.",
    )
    .example("420777123456"),
    Field::choice(
        "gateway",
        &["high", "economy", "low"],
        "Message route (price and quality level).",
    )
    .default("high"),
];

/// Uptime Kuma sends ASCII only; diacritics would need the Unicode type.
fn ascii(text: &str) -> String {
    text.chars().filter(char::is_ascii).collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    Ok(vec![HttpRequest::post(
        "https://http-api-lts.smsmanager.cz/",
    )
    .form([
        ("apikey", s.str("api_key")?.to_string()),
        ("number", s.required_list("numbers")?.join(";")),
        ("message", ascii(&m.text())),
        ("gateway", s.str("gateway")?.to_string()),
    ])
    .accept(classify)
    .into()])
}

/// Plain-text answer: `OK|<message id>|<number>` per recipient, or `ERROR|<code>`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let body = String::from_utf8_lossy(&r.body);
    let body = body.trim();
    if body.starts_with("OK") {
        Verdict::Accepted
    } else {
        Verdict::Rejected(r.summary())
    }
}
