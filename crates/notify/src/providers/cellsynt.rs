use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("cellsynt", "Cellsynt", FIELDS, build)
    .docs("https://www.cellsynt.com/en/sms/api-integration")
    .identity(&["destination"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("username", "Cellsynt account username."),
    Field::secret("password", "Cellsynt account password."),
    Field::text(
        "destination",
        "Recipient number in international format with a 00 prefix.",
    )
    .example("0046701234567"),
    Field::choice(
        "originator_type",
        &["alpha", "numeric"],
        "`alpha` sends from a name (recipients cannot reply); `numeric` from a phone number.",
    )
    .default("alpha"),
    Field::text(
        "originator",
        "Sender shown to the recipient: up to 11 letters and digits, or up to 15 digits when numeric.",
    )
    .default("Vesper"),
    Field::bool(
        "allow_long_sms",
        "Allow up to 6 concatenated messages instead of cutting the text to one 160-character SMS.",
    ),
];

/// Cellsynt's default character set is ISO-8859-1; Uptime Kuma sends ASCII only.
fn ascii(text: &str) -> String {
    text.chars().filter(char::is_ascii).collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let long = s.bool("allow_long_sms");
    let text: String = ascii(&m.text())
        .chars()
        .take(if long { 918 } else { 160 })
        .collect();
    Ok(vec![HttpRequest::post("https://se-1.cellsynt.net/sms.php")
        .form([
            ("username", s.str("username")?.to_string()),
            ("password", s.str("password")?.to_string()),
            ("destination", s.str("destination")?.to_string()),
            ("type", "text".to_string()),
            ("text", text),
            ("originatortype", s.str("originator_type")?.to_string()),
            ("originator", s.str("originator")?.to_string()),
            ("allowconcat", if long { "6" } else { "1" }.to_string()),
        ])
        .accept(classify)
        .into()])
}

/// Answers `OK: <tracking id>` or `Error: <description>` with status 200.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let body = String::from_utf8_lossy(&r.body);
    let body = body.trim();
    if body.starts_with("OK") {
        Verdict::Accepted
    } else if let Some(error) = body.strip_prefix("Error:") {
        Verdict::Rejected(error.trim().chars().take(120).collect())
    } else {
        Verdict::Transient(r.summary())
    }
}
