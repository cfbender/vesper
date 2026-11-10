use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("46elks", "46elks", FIELDS, build)
    .docs("https://46elks.com/docs/send-sms")
    .identity(&["to_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("username", "API username from https://46elks.com/account.")
        .example("u0123456789abcdef0123456789abcdef"),
    Field::secret("password", "API password from https://46elks.com/account."),
    Field::text(
        "from_number",
        "Text sender ID of up to 11 characters, or an E.164 number to be able to receive replies.",
    )
    .example("Vesper"),
    Field::text("to_number", "Recipient phone number in E.164 format.").example("+46701234567"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    Ok(vec![HttpRequest::post("https://api.46elks.com/a1/sms")
        .basic(s.str("username")?, s.str("password")?)
        .form([
            ("from", s.str("from_number")?.to_string()),
            ("to", s.str("to_number")?.to_string()),
            ("message", m.text()),
        ])
        .into()])
}
