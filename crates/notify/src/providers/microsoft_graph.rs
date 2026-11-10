use super::prelude::*;

pub const PROVIDER: Provider = Provider::new(
    "microsoft_graph",
    "Microsoft Graph (Outlook mail)",
    FIELDS,
    build,
)
.docs("https://learn.microsoft.com/en-us/graph/api/user-sendmail")
.identity(&["mailbox", "to", "cc", "bcc"]);

const FIELDS: &[Field] = &[
    Field::text(
        "tenant_id",
        "Directory (tenant) ID of the Entra ID app registration.",
    )
    .example("00000000-0000-0000-0000-000000000000"),
    Field::text(
        "client_id",
        "Application (client) ID of the app registration.",
    )
    .example("00000000-0000-0000-0000-000000000000"),
    Field::secret(
        "client_secret",
        "Client secret of the app registration, which needs the Mail.Send application permission.",
    ),
    Field::text(
        "mailbox",
        "Mailbox (user principal name or address) that sends the mail.",
    )
    .example("alerts@example.com"),
    Field::list("to", "Recipients.").example("ops@example.com"),
    Field::list("cc", "Copy recipients.").optional(),
    Field::list("bcc", "Blind copy recipients.").optional(),
];

fn recipients(addresses: Vec<String>) -> Value {
    addresses
        .into_iter()
        .map(|address| json!({ "emailAddress": { "address": address } }))
        .collect()
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let token = HttpRequest::post(format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        encode(s.str("tenant_id")?)
    ))
    .form([
        ("client_id", s.str("client_id")?),
        ("client_secret", s.str("client_secret")?),
        ("scope", "https://graph.microsoft.com/.default"),
        ("grant_type", "client_credentials"),
    ]);
    let body = json!({
        "message": {
            "subject": m.title,
            "body": { "contentType": "Text", "content": m.body() },
            "toRecipients": recipients(s.required_list("to")?),
            "ccRecipients": recipients(s.list("cc")),
            "bccRecipients": recipients(s.list("bcc")),
        },
        "saveToSentItems": false,
    });
    let url = format!(
        "https://graph.microsoft.com/v1.0/users/{}/sendMail",
        encode(s.str("mailbox")?)
    );
    let request = HttpRequest::post(url)
        .json(body)
        .bearer_from(token, "/access_token");
    Ok(vec![request.into()])
}
