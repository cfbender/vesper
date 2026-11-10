use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("home_assistant", "Home Assistant", FIELDS, build)
    .docs("https://developers.home-assistant.io/docs/api/rest/")
    .identity(&["url", "service"]);

const FIELDS: &[Field] = &[
    Field::url("url", "Home Assistant base URL.").example("https://homeassistant.example.com:8123"),
    Field::secret(
        "access_token",
        "Long-lived access token, created at the bottom of your Home Assistant profile page.",
    ),
    Field::text(
        "service",
        "Notify action to call, such as `mobile_app_my_phone`; `notify` reaches all devices.",
    )
    .default("notify")
    .example("mobile_app_my_phone"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let service = s.str("service")?;
    let service = service.strip_prefix("notify.").unwrap_or(service);
    if service.is_empty()
        || !service
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(
            "setting service must be a notify action name such as mobile_app_my_phone".into(),
        );
    }
    let mut body = json!({
        "title": m.title,
        "message": m.body(),
    });
    // persistent_notification rejects a data object.
    if service != "persistent_notification" {
        let mut data = json!({
            "channel": "Vesper",
            "url": m.link,
            "clickAction": m.link,
        });
        // Same values Uptime Kuma sends, for automations: 0 = down, 1 = up.
        match m.status {
            Status::Down => data["status"] = json!(0),
            Status::Up => data["status"] = json!(1),
            Status::Test => {}
        }
        body["data"] = data;
    }
    let url = join(
        &s.base_url("url")?,
        &format!("api/services/notify/{service}"),
    );
    Ok(vec![HttpRequest::post(url)
        .bearer(s.str("access_token")?)
        .json(body)
        .into()])
}
