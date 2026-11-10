use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("smtp", "Email (SMTP)", FIELDS, build)
    .docs("https://github.com/cfbender/vesper/blob/main/docs/notifications.md#smtp")
    .identity(&["host", "from", "to", "cc", "bcc"]);

const FIELDS: &[Field] = &[
    Field::text("host", "SMTP server host name or address.").example("smtp.example.com"),
    Field::int("port", "SMTP port.").default("587"),
    Field::choice(
        "security",
        &["starttls", "tls", "none"],
        "`starttls` upgrades with STARTTLS (port 587), `tls` is implicit TLS (port 465), `none` is plain SMTP for a trusted relay.",
    )
    .default("starttls"),
    Field::text("username", "Login user name.").optional(),
    Field::secret("password", "Login password.").optional(),
    Field::text("from", "Sender, such as `Vesper <vesper@example.com>`.")
        .example("Vesper <vesper@example.com>"),
    Field::list("to", "Recipients.").example("ops@example.com"),
    Field::list("cc", "Copy recipients.").optional(),
    Field::list("bcc", "Blind copy recipients.").optional(),
    Field::text("subject_prefix", "Text before the subject, such as `[homelab]`.").optional(),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let port = s.int("port")?.unwrap_or(587);
    let port = u16::try_from(port)
        .ok()
        .filter(|p| *p != 0)
        .ok_or("setting port must be between 1 and 65535")?;
    let security = match s.str("security")? {
        "tls" => MailSecurity::Tls,
        "none" => MailSecurity::None,
        _ => MailSecurity::StartTls,
    };
    let subject = match s.get("subject_prefix") {
        Some(prefix) => format!("{prefix} {}", m.title),
        None => m.title.clone(),
    };
    Ok(vec![MailRequest {
        host: s.str("host")?.to_string(),
        port,
        security,
        username: s.get("username").map(str::to_string),
        password: s.get("password").map(str::to_string),
        from: s.str("from")?.to_string(),
        to: s.required_list("to")?,
        cc: s.list("cc"),
        bcc: s.list("bcc"),
        subject,
        body: m.body(),
    }
    .into()])
}
