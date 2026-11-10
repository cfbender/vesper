use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("notifyapp", "Notify!", FIELDS, build)
    .docs("https://getnotifyapp.com/apidocs")
    .identity(&["device_id"]);

const FIELDS: &[Field] = &[
    Field::text(
        "device_id",
        "Device ID, or group ID (GRP…), from the Devices tab of the Notify! app.",
    )
    .example("ABC12345"),
    Field::secret("token", "Token belonging to the device or group ID."),
    Field::url(
        "icon_url",
        "HTTPS URL of a small sender icon shown next to the title.",
    )
    .optional(),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = json!({
        "title": m.title,
        "text": m.body(),
    });
    if let Some(icon) = s.get("icon_url") {
        body["iconUrl"] = json!(icon);
    }
    // Breaks through Focus and quiet hours.
    if m.status == Status::Down {
        body["timeSensitive"] = json!(true);
    }
    let url = format!(
        "https://push.getnotifyapp.com/notify-json/{}",
        encode(s.str("device_id")?)
    );
    let request = HttpRequest::post(url)
        .query([("token", s.str("token")?)])?
        .json(body);
    Ok(vec![request.into()])
}
