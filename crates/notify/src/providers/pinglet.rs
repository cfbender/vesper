use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pinglet", "Pinglet", FIELDS, build)
    .docs("https://pinglet.dev/docs/publishing/")
    .identity(&["topic_url"])
    .max_chars(3000);

const FIELDS: &[Field] = &[
    Field::url(
        "topic_url",
        "Topic URL to publish to, namespace and topic name; the topic is created on first publish.",
    )
    .example("https://pinglet.dev/acme/uptime"),
    Field::secret("api_key", "Namespace API key, starting with pinglet_.")
        .example("pinglet_0123456789abcdef"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let (priority, level) = match m.status {
        Status::Down => ("urgent", "error"),
        Status::Up => ("normal", "success"),
        Status::Test => ("normal", "info"),
    };
    let body = json!({
        "title": m.title,
        "message": m.body(),
        "priority": priority,
        "level": level,
    });
    Ok(vec![HttpRequest::post(s.base_url("topic_url")?)
        .bearer(s.str("api_key")?)
        .json(body)
        .into()])
}
