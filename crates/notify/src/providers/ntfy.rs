use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("ntfy", "ntfy", FIELDS, build)
    .docs("https://docs.ntfy.sh/publish/#publish-as-json")
    .identity(&["server_url", "topic"]);

const FIELDS: &[Field] = &[
    Field::url("server_url", "ntfy server.").default("https://ntfy.sh"),
    Field::text(
        "topic",
        "Topic name. On a public server anyone who knows it can read it; use a secret reference to keep it private.",
    )
    .example("vesper-alerts"),
    Field::choice(
        "priority",
        &["1", "2", "3", "4", "5"],
        "Priority of problems and tests; recoveries use one less.",
    )
    .default("4"),
    Field::text("username", "Username for an access-controlled topic.").optional(),
    Field::secret("password", "Password for the username.").optional(),
    Field::secret("access_token", "Access token, instead of username and password.").optional(),
    Field::url("icon", "Notification icon URL.").optional(),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let priority = s.int("priority")?.unwrap_or(4).clamp(1, 5);
    let (priority, tag) = match m.status {
        Status::Down => (priority, "red_circle"),
        Status::Up => ((priority - 1).max(1), "green_circle"),
        Status::Test => (priority, "test_tube"),
    };
    let mut body = json!({
        "topic": s.str("topic")?,
        "title": m.title,
        "message": m.summary(),
        "priority": priority,
        "tags": [tag],
        "click": m.link,
    });
    if let Some(icon) = s.get("icon") {
        body["icon"] = json!(icon);
    }
    let mut request = HttpRequest::post(s.base_url("server_url")?).json(body);
    if let Some(token) = s.get("access_token") {
        request = request.bearer(token);
    } else if let Some(user) = s.get("username") {
        request = request.basic(user, s.get("password").unwrap_or_default());
    }
    Ok(vec![request.into()])
}
