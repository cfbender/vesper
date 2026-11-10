use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pushy", "Pushy", FIELDS, build)
    .docs("https://pushy.me/docs/api/send-notifications")
    .identity(&["device_token"]);

const FIELDS: &[Field] = &[
    Field::secret(
        "api_key",
        "Secret API key of your Pushy app, from the Pushy dashboard.",
    )
    .example("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"),
    Field::text(
        "device_token",
        "Device token of the recipient, or a topic such as /topics/alerts.",
    )
    .example("cdd92f4ce847efa5c7f"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let body = json!({
        "to": s.str("device_token")?,
        "data": {
            "title": m.title,
            "message": m.summary(),
            "url": m.link,
        },
        "notification": {
            "title": m.title,
            "body": m.body(),
            "badge": 1,
            "sound": "ping.aiff",
        },
    });
    let request = HttpRequest::post("https://api.pushy.me/push")
        .query([("api_key", s.str("api_key")?)])?
        .json(body);
    Ok(vec![request.into()])
}
