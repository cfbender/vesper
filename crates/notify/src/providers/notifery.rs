use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("notifery", "Notifery", FIELDS, build)
    .docs("https://docs.notifery.com/api/event/")
    .identity(&["api_key", "group"])
    .max_chars(2000);

const FIELDS: &[Field] = &[
    Field::secret("api_key", "API key from the Notifery zone settings."),
    Field::text("group", "Group identifier the events are filed under.")
        .optional()
        .example("vesper"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = json!({
        "title": truncate(&m.title, 255),
        "message": truncate(&m.body(), 2048),
    });
    // A code above 0 marks an error, which Notifery can push.
    match m.status {
        Status::Down => body["code"] = json!(1),
        Status::Up => body["code"] = json!(0),
        Status::Test => {}
    }
    if let Some(group) = s.get("group") {
        body["group"] = json!(group);
    }
    Ok(vec![HttpRequest::post("https://api.notifery.com/event")
        .header("x-api-key", s.str("api_key")?)
        .json(body)
        .into()])
}
