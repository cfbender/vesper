use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("smseagle", "SMSEagle", FIELDS, build)
    .docs("https://www.smseagle.eu/docs/apiv2/")
    .identity(&["server_url", "recipient_type", "recipients"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Address of the SMSEagle device.")
        .example("https://smseagle.example.lan"),
    Field::secret(
        "access_token",
        "API access token created in the device's web GUI.",
    ),
    Field::choice(
        "api_version",
        &["v2", "v1"],
        "`v2` for current firmware, `v1` for the legacy HTTP API.",
    )
    .default("v2"),
    Field::choice(
        "recipient_type",
        &["phone", "group", "contact"],
        "Whether recipients are phone numbers, phonebook groups or phonebook contacts.",
    )
    .default("phone"),
    Field::list(
        "recipients",
        "Phone numbers in international format, or group or contact IDs (APIv2) or names (APIv1).",
    )
    .example("+48123456789"),
    Field::choice(
        "message_type",
        &["sms", "ring", "tts", "tts_advanced"],
        "Send an SMS, ring only, or call and read the message aloud (text-to-speech).",
    )
    .default("sms"),
    Field::int(
        "priority",
        "SMS priority from 0 to 9; APIv1 treats any value above 0 as high.",
    )
    .default("0"),
    Field::bool("unicode", "Encode SMS as Unicode for national characters."),
    Field::int(
        "duration",
        "Call duration in seconds, for ring and text-to-speech calls.",
    )
    .default("10"),
    Field::int(
        "voice_id",
        "Voice model ID for advanced text-to-speech calls.",
    )
    .default("1"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let base = s.base_url("server_url")?;
    let token = s.str("access_token")?;
    let recipients = s.required_list("recipients")?;
    let recipient_type = s.str("recipient_type")?;
    let message_type = s.str("message_type")?;
    let priority = s.int("priority")?.unwrap_or(0).clamp(0, 9);
    let duration = s.int("duration")?.unwrap_or(10).clamp(1, 30);
    let voice_id = s.int("voice_id")?.unwrap_or(1);
    let text = m.text();

    if s.str("api_version")? == "v1" {
        return v1(
            s,
            &base,
            token,
            &recipients,
            recipient_type,
            message_type,
            priority,
            duration,
            voice_id,
            &text,
        );
    }

    let mut body = json!({});
    match recipient_type {
        "phone" => body["to"] = json!(recipients),
        kind => {
            let ids = recipients
                .iter()
                .map(|r| r.parse::<i64>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| format!("APIv2 {kind} recipients must be numeric IDs"))?;
            body[if kind == "group" {
                "groups"
            } else {
                "contacts"
            }] = json!(ids);
        }
    }
    let endpoint = match message_type {
        "ring" => "calls/ring",
        "tts" => "calls/tts",
        "tts_advanced" => "calls/tts_advanced",
        _ => "messages/sms",
    };
    if message_type == "sms" {
        body["text"] = json!(text);
        body["encoding"] = json!(if s.bool("unicode") {
            "unicode"
        } else {
            "standard"
        });
        body["priority"] = json!(priority);
    } else {
        body["duration"] = json!(duration);
        if message_type != "ring" {
            body["text"] = json!(text);
        }
        if message_type == "tts_advanced" {
            body["voice_id"] = json!(voice_id);
        }
    }
    let request = HttpRequest::post(join(&base, &format!("api/v2/{endpoint}")))
        .header("access-token", token)
        .json(body)
        .accept(accept_v2);
    Ok(vec![request.into()])
}

/// The legacy HTTP API takes one recipient kind per call and the token as a
/// query parameter; calls only go to phone numbers.
#[allow(clippy::too_many_arguments)]
fn v1(
    s: &Settings,
    base: &str,
    token: &str,
    recipients: &[String],
    recipient_type: &str,
    message_type: &str,
    priority: i64,
    duration: i64,
    voice_id: i64,
    text: &str,
) -> Result<Vec<Request>, String> {
    let high = if priority > 0 { "1" } else { "0" };
    let unicode = if s.bool("unicode") { "1" } else { "0" };
    let sms_params = |key: &str, value: &str| -> Vec<(String, String)> {
        vec![
            ("access_token".into(), token.into()),
            (key.into(), value.into()),
            ("message".into(), text.into()),
            ("highpriority".into(), high.into()),
            ("unicode".into(), unicode.into()),
        ]
    };
    let get = |method: &str, params: Vec<(String, String)>| -> Result<Request, String> {
        Ok(HttpRequest::get(join(base, &format!("http_api/{method}")))
            .query(params)?
            .accept(accept_v1)
            .into())
    };
    match recipient_type {
        "group" | "contact" => {
            if message_type != "sms" {
                return Err("APIv1 calls only go to phone numbers".into());
            }
            let (method, key) = if recipient_type == "group" {
                ("send_togroup", "groupname")
            } else {
                ("send_tocontact", "contactname")
            };
            recipients
                .iter()
                .map(|name| get(method, sms_params(key, name)))
                .collect()
        }
        _ => {
            let to = recipients.join(",");
            if message_type == "sms" {
                return Ok(vec![get("send_sms", sms_params("to", &to))?]);
            }
            let mut params = vec![
                ("access_token".to_string(), token.to_string()),
                ("to".into(), to),
                ("duration".into(), duration.to_string()),
            ];
            let method = match message_type {
                "ring" => "ring_call",
                "tts" => "tts_call",
                _ => "tts_adv_call",
            };
            if message_type != "ring" {
                params.push(("message".into(), text.into()));
            }
            if message_type == "tts_advanced" {
                params.push(("voice_id".into(), voice_id.to_string()));
            }
            Ok(vec![get(method, params)?])
        }
    }
}

/// APIv1 answers `OK; ID=[…]` on success and an error text (such as
/// `Invalid login or password` or `Wrong TOKEN`) otherwise.
fn accept_v1(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted if String::from_utf8_lossy(&r.body).trim().starts_with("OK") => {
            Verdict::Accepted
        }
        Verdict::Accepted => Verdict::Rejected(r.summary()),
        other => other,
    }
}

/// APIv2 answers with one entry per recipient; at least one must be queued.
fn accept_v2(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {
            let queued = r.json().as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|i| i.get("status").and_then(Value::as_str) == Some("queued"))
            });
            if queued {
                Verdict::Accepted
            } else {
                Verdict::Rejected(r.summary())
            }
        }
        other => other,
    }
}
