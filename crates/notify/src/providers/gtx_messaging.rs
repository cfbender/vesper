use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("gtx_messaging", "GTX Messaging", FIELDS, build)
    .docs("https://www.gtx-messaging.com/en/api-docs/sms-rest-api")
    .identity(&["to"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key from the GTX dashboard."),
    Field::text(
        "from",
        "Sender: a phone number or alphanumeric originator (TPOA).",
    )
    .example("Vesper"),
    Field::text("to", "Recipient phone number in E.164 format.").example("+4917612345678"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = format!(
        "https://rest.gtx-messaging.net/smsc/sendsms/{}/json",
        encode(s.str("api_key")?)
    );
    Ok(vec![HttpRequest::post(url)
        .header("Accept", "application/json")
        .form([
            ("from", s.str("from")?.to_string()),
            ("to", s.str("to")?.to_string()),
            ("text", m.text()),
        ])
        .into()])
}
