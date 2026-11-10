use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("smsc", "SMSC", FIELDS, build)
    .docs("https://smsc.kz/api/http/send")
    .identity(&["to_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("login", "SMSC account login."),
    Field::secret("password", "SMSC account password or API password."),
    Field::text(
        "to_number",
        "Recipient phone number in international format; separate several with commas.",
    )
    .example("77011234567"),
    Field::text(
        "sender_name",
        "Registered sender name of up to 15 characters; leave empty for the shared sender.",
    )
    .optional()
    .example("Vesper"),
    Field::choice(
        "translit",
        &["0", "1", "2"],
        "0 sends the text as is, 1 transliterates Cyrillic to Latin (translit), 2 to look-alike Latin letters.",
    )
    .default("0"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut form = vec![
        ("login", s.str("login")?.to_string()),
        ("psw", s.str("password")?.to_string()),
        ("phones", s.str("to_number")?.to_string()),
        ("mes", m.text()),
        ("translit", s.str("translit")?.to_string()),
        ("charset", "utf-8".to_string()),
        ("fmt", "3".to_string()),
    ];
    if let Some(sender) = s.get("sender_name") {
        form.push(("sender", sender.to_string()));
    }
    Ok(vec![HttpRequest::post("https://smsc.kz/sys/send.php")
        .form(form)
        .accept(classify)
        .into()])
}

/// With `fmt=3` the answer is `{"id":…,"cnt":…}` or
/// `{"error":"…","error_code":N}`. Code 9 means too many identical or
/// concurrent requests, 4 a temporarily blocked IP address.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let json = r.json();
    let error = json
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match json.get("error_code").and_then(Value::as_u64) {
        None if json.get("id").is_some() => Verdict::Accepted,
        None => Verdict::Transient(r.summary()),
        Some(9) => Verdict::RetryAfter(60),
        Some(4) => Verdict::Transient(format!("error 4: {error}")),
        Some(code) => Verdict::Rejected(format!("error {code}: {error}")),
    }
}
