use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pushover", "Pushover", FIELDS, build)
    .docs("https://pushover.net/api")
    .identity(&["user_key", "device"])
    .max_chars(1024);

const SOUNDS: &[&str] = &[
    "pushover",
    "bike",
    "bugle",
    "cashregister",
    "classical",
    "cosmic",
    "falling",
    "gamelan",
    "incoming",
    "intermission",
    "magic",
    "mechanical",
    "pianobar",
    "siren",
    "spacealarm",
    "tugboat",
    "alien",
    "climb",
    "persistent",
    "echo",
    "updown",
    "vibrate",
    "none",
];

const FIELDS: &[Field] = &[
    Field::text("user_key", "User or group key of the recipient, from the Pushover dashboard.")
        .example("uQiRzpo4DXghDmr9QzzfQu27cmVRsG"),
    Field::secret("app_token", "API token of your Pushover application.")
        .example("azGDORePK8gMaC0QOYAMyEEuzJnyUi"),
    Field::text("device", "Send only to this device name instead of all the user's devices.")
        .optional()
        .example("phone"),
    Field::choice(
        "priority",
        &["-2", "-1", "0", "1", "2"],
        "Priority from -2 (silent) to 2 (emergency, repeated until acknowledged); recoveries never use emergency.",
    )
    .default("0"),
    Field::choice("sound", SOUNDS, "Sound for problems and tests; the user's default when unset.")
        .optional(),
    Field::choice("sound_up", SOUNDS, "Sound for recoveries; the problem sound when unset.").optional(),
    Field::int("ttl", "Seconds after which the message is deleted from devices.").optional(),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut priority = s.int("priority")?.unwrap_or(0).clamp(-2, 2);
    if m.status == Status::Up {
        // A recovery should not page until acknowledged.
        priority = priority.min(1);
    }
    let mut body = json!({
        "token": s.str("app_token")?,
        "user": s.str("user_key")?,
        "title": truncate(&m.title, 250),
        "message": truncate(&m.summary(), 1024),
        "url": m.link,
        "url_title": "Open in Vesper",
        "priority": priority,
        "timestamp": m.at.timestamp(),
    });
    if priority == 2 {
        body["retry"] = json!(30);
        body["expire"] = json!(3600);
    }
    let sound = match m.status {
        Status::Up => s.get("sound_up").or(s.get("sound")),
        _ => s.get("sound"),
    };
    if let Some(sound) = sound {
        body["sound"] = json!(sound);
    }
    if let Some(device) = s.get("device") {
        body["device"] = json!(device);
    }
    if let Some(ttl) = s.int("ttl")? {
        if ttl <= 0 {
            return Err("setting ttl must be positive".into());
        }
        body["ttl"] = json!(ttl);
    }
    Ok(vec![HttpRequest::post(
        "https://api.pushover.net/1/messages.json",
    )
    .json(body)
    .into()])
}
