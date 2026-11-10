use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("bark", "Bark", FIELDS, build)
    .docs("https://bark.day.app/en-us/tutorial")
    .identity(&["endpoint"]);

const SOUNDS: &[&str] = &[
    "alarm",
    "anticipate",
    "bell",
    "birdsong",
    "bloom",
    "calypso",
    "chime",
    "choo",
    "descent",
    "electronic",
    "fanfare",
    "glass",
    "gotosleep",
    "healthnotification",
    "horn",
    "ladder",
    "mailsent",
    "minuet",
    "multiwayinvitation",
    "newmail",
    "newsflash",
    "noir",
    "paymentsuccess",
    "shake",
    "sherwoodforest",
    "silence",
    "spell",
    "suspense",
    "telegraph",
    "tiptoes",
    "typewriters",
    "update",
];

const FIELDS: &[Field] = &[
    Field::secret_url(
        "endpoint",
        "Push URL with the device key, as shown in the Bark app, on api.day.app or your own bark-server.",
    )
    .example("https://api.day.app/ynJ5Ft4atkMkWeo2PAvFhF"),
    Field::choice(
        "api_version",
        &["v1", "v2"],
        "`v1` sends a GET with query parameters; `v2` posts JSON (bark-server 2 and later).",
    )
    .default("v1"),
    Field::text("group", "Group the notifications are filed under on the device.").default("Vesper"),
    Field::choice("sound", SOUNDS, "Notification sound.").default("telegraph"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let endpoint = s.base_url("endpoint")?;
    let title = m.title.clone();
    let body = m.summary();
    let group = s.str("group")?;
    let sound = s.str("sound")?;
    let request = match s.str("api_version")? {
        "v2" => HttpRequest::post(endpoint).json(json!({
            "title": title,
            "body": body,
            "group": group,
            "sound": sound,
            "url": m.link,
        })),
        // Query parameters rather than path segments, so a `/` in the text
        // cannot change the route.
        _ => HttpRequest::get(endpoint).query([
            ("title", title.as_str()),
            ("body", body.as_str()),
            ("group", group),
            ("sound", sound),
            ("url", m.link.as_str()),
        ])?,
    };
    Ok(vec![request.into()])
}
