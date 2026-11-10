use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("sendgrid", "SendGrid", FIELDS, build)
    .docs("https://www.twilio.com/docs/sendgrid/api-reference/mail-send/mail-send")
    .identity(&["from_email", "to", "cc", "bcc"]);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key with the Mail Send permission."),
    Field::text("from_email", "Verified sender address.").example("vesper@example.com"),
    Field::list("to", "Recipients.").example("ops@example.com"),
    Field::list("cc", "Copy recipients.").optional(),
    Field::list("bcc", "Blind copy recipients.").optional(),
    Field::text(
        "subject_prefix",
        "Text before the subject, such as `[homelab]`.",
    )
    .optional(),
];

fn addresses(list: Vec<String>) -> Value {
    list.into_iter()
        .map(|email| json!({ "email": email }))
        .collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let subject = match s.get("subject_prefix") {
        Some(prefix) => format!("{prefix} {}", m.title),
        None => m.title.clone(),
    };
    let mut personalization = json!({ "to": addresses(s.required_list("to")?) });
    let (cc, bcc) = (s.list("cc"), s.list("bcc"));
    if !cc.is_empty() {
        personalization["cc"] = addresses(cc);
    }
    if !bcc.is_empty() {
        personalization["bcc"] = addresses(bcc);
    }
    let body = json!({
        "personalizations": [personalization],
        "from": { "email": s.str("from_email")? },
        "subject": subject,
        "content": [{ "type": "text/plain", "value": m.body() }],
    });
    let request = HttpRequest::post("https://api.sendgrid.com/v3/mail/send")
        .bearer(s.str("api_key")?)
        .json(body);
    Ok(vec![request.into()])
}
