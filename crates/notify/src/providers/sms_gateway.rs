use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("sms_gateway", "SMS Gateway", FIELDS, build)
    .docs("https://github.com/mattboston/sms-gateway")
    .identity(&["server_url", "to"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Address of the self-hosted SMS Gateway.")
        .example("http://sms-gateway.lan:8080"),
    Field::secret("api_key", "API key of the gateway."),
    Field::list("to", "Recipient phone numbers in international format.").example("+15551234567"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = join(&s.base_url("server_url")?, "api/v1/sms/send");
    let key = s.str("api_key")?;
    let text = m.text();
    Ok(s.required_list("to")?
        .into_iter()
        .map(|to| {
            HttpRequest::post(url.clone())
                .header("X-API-Key", key)
                .json(json!({ "to": to, "body": text }))
                .accept(classify)
                .into()
        })
        .collect())
}

/// The gateway answers 200 even when the modem could not send, with
/// `status: failed` in the body.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted if r.json().get("status").and_then(Value::as_str) == Some("failed") => {
            Verdict::Transient(format!("modem could not send: {}", r.summary()))
        }
        other => other,
    }
}
