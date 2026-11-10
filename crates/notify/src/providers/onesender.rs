use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("onesender", "OneSender", FIELDS, build)
    .docs("https://onesender.net/docs/")
    .identity(&["url", "recipient_type", "receiver"]);

const FIELDS: &[Field] = &[
    Field::url("url", "Messages endpoint of your OneSender instance.")
        .example("https://onesender.example.com/api/v1/messages"),
    Field::secret("token", "OneSender API token."),
    Field::choice(
        "recipient_type",
        &["private", "group"],
        "Send to a private number or to a group.",
    )
    .default("private"),
    Field::text(
        "receiver",
        "Phone number with country code (such as `628123456789`) or group ID (such as `628123456789-1473638645`), without the `@` suffix.",
    )
    .example("628123456789"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let receiver = s.str("receiver")?;
    let (recipient_type, to) = match s.str("recipient_type")? {
        "group" => ("group", format!("{receiver}@g.us")),
        _ => ("individual", format!("{receiver}@s.whatsapp.net")),
    };
    let request = HttpRequest::post(s.str("url")?)
        .bearer(s.str("token")?)
        .json(json!({
            "to": to,
            "type": "text",
            "recipient_type": recipient_type,
            "text": { "body": m.text() },
        }));
    Ok(vec![request.into()])
}
