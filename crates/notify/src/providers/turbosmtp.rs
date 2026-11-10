use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("turbosmtp", "turboSMTP", FIELDS, build)
    .docs("https://serversmtp.com/docs/api-reference/overview")
    .identity(&["from_email", "to", "cc", "bcc"]);

const FIELDS: &[Field] = &[
    Field::secret(
        "consumer_key",
        "Consumer key with HTTP API sending permission.",
    ),
    Field::secret("consumer_secret", "Consumer secret of the key pair."),
    Field::choice(
        "region",
        &["us", "eu"],
        "Sending infrastructure: `us` (global) or `eu`.",
    )
    .default("us"),
    Field::text("from_email", "Sender, an address or `Name <address>`.")
        .example("vesper@example.com"),
    Field::list("to", "Recipients.").example("ops@example.com"),
    Field::list("cc", "Copy recipients.").optional(),
    Field::list("bcc", "Blind copy recipients.").optional(),
    Field::text(
        "subject_prefix",
        "Text before the subject, such as `[homelab]`.",
    )
    .optional(),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let host = match s.str("region")? {
        "eu" => "api.eu.turbo-smtp.com",
        _ => "api.turbo-smtp.com",
    };
    let subject = match s.get("subject_prefix") {
        Some(prefix) => format!("{prefix} {}", m.title),
        None => m.title.clone(),
    };
    // Recipient fields are comma-separated strings.
    let mut body = json!({
        "from": s.str("from_email")?,
        "to": s.required_list("to")?.join(","),
        "subject": subject,
        "content": m.body(),
    });
    let (cc, bcc) = (s.list("cc"), s.list("bcc"));
    if !cc.is_empty() {
        body["cc"] = json!(cc.join(","));
    }
    if !bcc.is_empty() {
        body["bcc"] = json!(bcc.join(","));
    }
    let request = HttpRequest::post(format!("https://{host}/api/v2/mail/send"))
        .header("consumerKey", s.str("consumer_key")?)
        .header("consumerSecret", s.str("consumer_secret")?)
        .json(body);
    Ok(vec![request.into()])
}
