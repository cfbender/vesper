use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("evolution", "Evolution API", FIELDS, build)
    .docs("https://docs.evoapicloud.com/api-reference/message-controller/send-text")
    .identity(&["server_url", "instance_name", "recipient"]);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Address of the Evolution API server.")
        .example("https://evolution.example.com"),
    Field::text("instance_name", "Name of the connected WhatsApp instance.").example("vesper"),
    Field::secret("auth_token", "API key of the instance (`apikey` header)."),
    Field::text(
        "recipient",
        "Phone number with country code and no `+`, or a group JID such as `120363000000000000@g.us`.",
    )
    .example("5511999999999"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = join(
        &s.base_url("server_url")?,
        &format!("message/sendText/{}", encode(s.str("instance_name")?)),
    );
    let request = HttpRequest::post(url)
        .header("apikey", s.str("auth_token")?)
        .header("Accept", "application/json")
        .json(json!({ "number": s.str("recipient")?, "text": m.text() }));
    Ok(vec![request.into()])
}
