use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("gorush", "Gorush", FIELDS, build)
    .docs("https://github.com/appleboy/gorush#web-api")
    .identity(&["server_url", "device_token"]);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Gorush server.").example("https://gorush.example.com"),
    Field::text("device_token", "Push token of the receiving device.")
        .example("a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90"),
    Field::choice(
        "platform",
        &["ios", "android", "huawei"],
        "Platform of the device: APNs, FCM or Huawei Push Kit.",
    ),
    Field::choice("priority", &["normal", "high"], "Delivery priority.").optional(),
    Field::int("retry", "How many times gorush retries a failed push.").default("0"),
    Field::text("topic", "APNs topic (app bundle ID) or FCM topic.")
        .optional()
        .example("com.example.app"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let platform = match s.str("platform")? {
        "ios" => 1,
        "android" => 2,
        "huawei" => 3,
        other => return Err(format!("unknown platform {other}")),
    };
    let retry = s.int("retry")?.unwrap_or(0);
    if retry < 0 {
        return Err("setting retry must not be negative".into());
    }
    let mut notification = json!({
        "tokens": [s.str("device_token")?],
        "platform": platform,
        "title": m.title,
        "message": m.body(),
        "retry": retry,
    });
    if let Some(priority) = s.get("priority") {
        notification["priority"] = json!(priority);
    }
    if let Some(topic) = s.get("topic") {
        notification["topic"] = json!(topic);
    }
    let url = join(&s.base_url("server_url")?, "api/push");
    Ok(vec![HttpRequest::post(url)
        .json(json!({ "notifications": [notification] }))
        .into()])
}
