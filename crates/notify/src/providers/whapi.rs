use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("whapi", "Whapi.Cloud", FIELDS, build)
    .docs("https://whapi.readme.io/reference/sendmessagetext")
    .identity(&["server_url", "recipient"]);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Whapi API gateway.").default("https://gate.whapi.cloud"),
    Field::secret("auth_token", "Channel API token from the Whapi dashboard."),
    Field::text(
        "recipient",
        "Phone number with country code and no `+`, or a chat or group ID such as `120363194020948049@g.us`.",
    )
    .example("34612345678"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = join(&s.base_url("server_url")?, "messages/text");
    let request = HttpRequest::post(url)
        .bearer(s.str("auth_token")?)
        .header("Accept", "application/json")
        .json(json!({ "to": s.str("recipient")?, "body": m.text() }));
    Ok(vec![request.into()])
}
