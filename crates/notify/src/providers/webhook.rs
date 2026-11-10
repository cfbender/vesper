use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("webhook", "Webhook", FIELDS, build)
    .docs("https://github.com/cfbender/vesper/blob/main/docs/notifications.md#webhook")
    .identity(&["url"]);

const FIELDS: &[Field] = &[
    Field::url("url", "Endpoint that receives the notification.")
        .example("https://hooks.example.com/vesper"),
    Field::choice(
        "content_type",
        &["json", "form"],
        "`json` posts the message object; `form` posts its fields form-encoded.",
    )
    .default("json"),
    Field::secret("authorization", "Value of the Authorization header.").optional(),
];

/// Posts `{title, text, lines, link, status, at, incident}`. `status` is
/// `down`, `up` or `test`; `incident` is null.
fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let status = m.status_word().to_lowercase();
    let mut request = HttpRequest::post(s.str("url")?);
    request = match s.str("content_type")? {
        "form" => request.form([
            ("title", m.title.clone()),
            ("text", m.text()),
            ("link", m.link.clone()),
            ("status", status),
            ("at", m.at.to_rfc3339()),
        ]),
        _ => request.json(json!({
            "title": m.title,
            "text": m.text(),
            "lines": m.lines,
            "link": m.link,
            "status": status,
            "at": m.at.to_rfc3339(),
            "incident": m.incident.as_ref().map(|i| json!({
                "id": i.id,
                "service_id": i.service_id,
                "service_name": i.service_name,
            })),
        })),
    };
    if let Some(auth) = s.get("authorization") {
        request = request.header("Authorization", auth);
    }
    Ok(vec![request.into()])
}
