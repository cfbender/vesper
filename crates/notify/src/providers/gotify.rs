use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("gotify", "Gotify", FIELDS, build)
    .docs("https://gotify.net/docs/pushmsg")
    .identity(&["server_url", "application_token"]);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Gotify server.").example("https://gotify.example.com"),
    Field::secret(
        "application_token",
        "Token of the application the messages are posted as, from Apps in the Gotify web UI.",
    )
    .example("AbCdEf0123456789"),
    Field::int(
        "priority",
        "Message priority from 0 (lowest) to 10 (highest).",
    )
    .default("8"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let priority = s.int("priority")?.unwrap_or(8);
    if !(0..=10).contains(&priority) {
        return Err("setting priority must be between 0 and 10".into());
    }
    let body = json!({
        "title": m.title,
        "message": m.body(),
        "priority": priority,
        "extras": {
            "client::notification": { "click": { "url": m.link } },
        },
    });
    let url = join(&s.base_url("server_url")?, "message");
    Ok(vec![HttpRequest::post(url)
        .header("X-Gotify-Key", s.str("application_token")?)
        .json(body)
        .into()])
}
