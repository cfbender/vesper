use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("brevo", "Brevo", FIELDS, build)
    .docs("https://developers.brevo.com/reference/sendtransacemail")
    .identity(&["from_email", "to", "cc", "bcc"]);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key from SMTP & API settings."),
    Field::text("from_email", "Verified sender address.").example("vesper@example.com"),
    Field::text("from_name", "Sender name.").default("Vesper"),
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

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let subject = match s.get("subject_prefix") {
        Some(prefix) => format!("{prefix} {}", m.title),
        None => m.title.clone(),
    };
    let lines: Vec<String> = m.lines.iter().map(|l| escape_html(l)).collect();
    let link = escape_html(&m.link);
    let html = format!(
        "<html><body><p>{}</p><p><a href=\"{link}\">{link}</a></p></body></html>",
        lines.join("<br>")
    );
    let mut body = json!({
        "sender": { "email": s.str("from_email")?, "name": s.str("from_name")? },
        "to": addresses(s.required_list("to")?),
        "subject": subject,
        "htmlContent": html,
        "textContent": m.body(),
    });
    let (cc, bcc) = (s.list("cc"), s.list("bcc"));
    if !cc.is_empty() {
        body["cc"] = addresses(cc);
    }
    if !bcc.is_empty() {
        body["bcc"] = addresses(bcc);
    }
    let request = HttpRequest::post("https://api.brevo.com/v3/smtp/email")
        .header("api-key", s.str("api_key")?)
        .header("Accept", "application/json")
        .json(body);
    Ok(vec![request.into()])
}
