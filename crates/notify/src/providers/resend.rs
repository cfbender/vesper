use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("resend", "Resend", FIELDS, build)
    .docs("https://resend.com/docs/api-reference/emails/send-email")
    .identity(&["from_email", "to"]);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key with sending access."),
    Field::text("from_email", "Sender address on a verified domain.").example("vesper@example.com"),
    Field::text("from_name", "Sender name.").default("Vesper"),
    Field::list("to", "Recipients, at most 50.").example("ops@example.com"),
    Field::text(
        "subject_prefix",
        "Text before the subject, such as `[homelab]`.",
    )
    .optional(),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let subject = match s.get("subject_prefix") {
        Some(prefix) => format!("{prefix} {}", m.title),
        None => m.title.clone(),
    };
    let body = json!({
        "from": format!("{} <{}>", s.str("from_name")?, s.str("from_email")?),
        "to": s.required_list("to")?,
        "subject": subject,
        "text": m.body(),
    });
    let request = HttpRequest::post("https://api.resend.com/emails")
        .bearer(s.str("api_key")?)
        .json(body);
    Ok(vec![request.into()])
}
