use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("fluxer", "Fluxer", FIELDS, build)
    .docs("https://docs.fluxer.app/http-api/webhooks")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] = &[
    Field::secret_url(
        "webhook_url",
        "Webhook URL from the Webhooks tab of the community or channel settings.",
    )
    .example("https://api.fluxer.app/webhooks/417429632418316298/abcdef"),
    Field::text("username", "Display name of the webhook.").default("Vesper"),
    Field::text(
        "prefix_message",
        "Text sent before the embed, such as a mention like `@everyone`.",
    )
    .optional(),
    Field::choice(
        "message_format",
        &["normal", "minimalist"],
        "`normal` posts an embed; `minimalist` posts plain text with a status emoji.",
    )
    .default("normal"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = json!({ "username": s.str("username")? });
    if s.str("message_format")? == "minimalist" {
        let emoji = match m.status {
            Status::Down => "🔴",
            Status::Up => "🟢",
            Status::Test => "🧪",
        };
        body["content"] = json!(truncate(&format!("{emoji} {}", m.text()), 2000));
    } else {
        let color = match m.status {
            Status::Down => 0xff0000,
            Status::Up => 0x00ff00,
            Status::Test => 0x5865f2,
        };
        body["embeds"] = json!([{
            "title": truncate(&m.title, 256),
            "description": truncate(&m.summary(), 4096),
            "url": m.link,
            "color": color,
            "timestamp": m.at.to_rfc3339(),
        }]);
        if let Some(prefix) = s.get("prefix_message") {
            body["content"] = json!(prefix);
        }
    }
    Ok(vec![HttpRequest::post(s.str("webhook_url")?)
        .json(body)
        .into()])
}
