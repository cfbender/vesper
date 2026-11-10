use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("teltonika", "Teltonika", FIELDS, build)
    .docs("https://developers.teltonika-networks.com/reference/rut241/7.19.4/v1.11.1/messages")
    .identity(&["url", "modem", "phone_number"])
    .max_chars(160);

const FIELDS: &[Field] = &[
    Field::url(
        "url",
        "Router address; only scheme, host and port are used (RutOS 7.14 or later).",
    )
    .example("https://192.168.1.1"),
    Field::text("username", "Router user allowed to send SMS.").example("smsuser"),
    Field::secret("password", "Password of that user."),
    Field::text("modem", "Modem ID, such as `1-1`.").example("1-1"),
    Field::text(
        "phone_number",
        "Recipient phone number in international format.",
    )
    .example("+37060000000"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let url = url::Url::parse(s.str("url")?).map_err(|_| "setting url is not a valid URL")?;
    let origin = url.origin();
    if !origin.is_tuple() {
        return Err("setting url needs a host".into());
    }
    let base = origin.ascii_serialization();
    let login = HttpRequest::post(format!("{base}/api/login"))
        .json(json!({
            "username": s.str("username")?,
            "password": s.str("password")?,
        }))
        .accept(classify);
    let request = HttpRequest::post(format!("{base}/api/messages/actions/send"))
        .json(json!({
            "data": {
                "modem": s.str("modem")?,
                "number": s.str("phone_number")?,
                "message": truncate(&m.text(), 160),
            }
        }))
        .accept(classify)
        .bearer_from(login, "/data/token");
    Ok(vec![request.into()])
}

/// The RutOS API answers `{success, data, errors}`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted if r.json().get("success").and_then(Value::as_bool) == Some(true) => {
            Verdict::Accepted
        }
        Verdict::Accepted => {
            let json = r.json();
            let error = json
                .pointer("/errors/0/error")
                .or_else(|| json.pointer("/errors/error"))
                .and_then(Value::as_str)
                .map_or_else(|| r.summary(), str::to_string);
            Verdict::Rejected(error)
        }
        other => other,
    }
}
