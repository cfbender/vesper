use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pushplus", "PushPlus", FIELDS, build)
    .docs("https://www.pushplus.plus/doc/guide/api.html")
    .identity(&["token", "topic"]);

const FIELDS: &[Field] = &[
    Field::secret("token", "User or message token from the PushPlus console.")
        .example("3cbc5eab19fe512e80677540fbde332a"),
    Field::text(
        "topic",
        "Group code to send to the group's members instead of only yourself.",
    )
    .optional()
    .example("ops"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut body = json!({
        "token": s.str("token")?,
        "title": truncate(&m.title, 100),
        "content": m.body(),
        "template": "txt",
    });
    if let Some(topic) = s.get("topic") {
        body["topic"] = json!(topic);
    }
    Ok(vec![HttpRequest::post("https://www.pushplus.plus/send")
        .json(body)
        .accept(classify)
        .into()])
}

/// PushPlus answers HTTP 200 with `code`: 200 accepted, 900 account rate
/// limited (requests resume after a while), 500 server error, anything else
/// a token, verification or validation problem.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let json = r.json();
    let code = json
        .get("code")
        .map(|c| c.as_str().map_or_else(|| c.to_string(), str::to_string));
    match code.as_deref() {
        Some("200") => Verdict::Accepted,
        Some("900") => Verdict::RetryAfter(3600),
        Some("500") | None => Verdict::Transient(r.summary()),
        Some(_) => Verdict::Rejected(r.summary()),
    }
}
