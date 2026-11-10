use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("threema", "Threema Gateway", FIELDS, build)
    .docs("https://gateway.threema.ch/en/developer/api")
    .identity(&["recipient_type", "recipient"])
    // Basic mode takes at most 3500 bytes of UTF-8 text.
    .max_chars(3000);

const FIELDS: &[Field] = &[
    Field::text(
        "sender_identity",
        "Gateway ID in basic mode, eight characters starting with `*`.",
    )
    .example("*VESPER1"),
    Field::secret("secret", "API authentication secret of the Gateway ID."),
    Field::choice(
        "recipient_type",
        &["identity", "phone", "email"],
        "Whether the recipient is a Threema ID, a phone number or an email address linked to one.",
    )
    .default("identity"),
    Field::text(
        "recipient",
        "Threema ID (eight characters), phone number in international format without `+` (such as `41791234567`), or email address.",
    )
    .example("ECHOECHO"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let key = match s.str("recipient_type")? {
        "phone" => "phone",
        "email" => "email",
        _ => "to",
    };
    let request = HttpRequest::post("https://msgapi.threema.ch/send_simple")
        .header("Accept", "*/*")
        .form([
            ("from", s.str("sender_identity")?),
            (key, s.str("recipient")?),
            ("secret", s.str("secret")?),
            ("text", &m.text()),
        ])
        .accept(classify);
    Ok(vec![request.into()])
}

/// Explains the documented status codes; 402 (no credits) is a
/// configuration problem.
fn classify(r: &Response) -> Verdict {
    let reason = match r.status {
        400 => "invalid recipient, or the ID is not set up for basic mode",
        401 => "wrong API identity or secret",
        402 => "no credits remaining",
        404 => "recipient not found",
        413 => "message too long",
        _ => return by_status(r),
    };
    Verdict::Rejected(format!("HTTP {}: {reason}", r.status))
}
