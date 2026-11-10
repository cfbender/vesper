use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("onebot", "OneBot", FIELDS, build)
    .docs("https://github.com/botuniverse/onebot-11/blob/master/api/public.md#send_msg-%E5%8F%91%E9%80%81%E6%B6%88%E6%81%AF")
    .identity(&["server_url", "message_type", "receiver_id"]);

const FIELDS: &[Field] = &[
    Field::url(
        "server_url",
        "HTTP API address of the OneBot 11 implementation.",
    )
    .example("http://127.0.0.1:5700"),
    Field::secret(
        "access_token",
        "Access token configured in the OneBot implementation.",
    )
    .optional(),
    Field::choice(
        "message_type",
        &["private", "group"],
        "Send to a user (`private`) or a group (`group`).",
    )
    .default("private"),
    Field::text("receiver_id", "QQ number of the user, or the group number.").example("123456789"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let receiver: i64 = s
        .str("receiver_id")?
        .parse()
        .map_err(|_| "setting receiver_id must be a number".to_string())?;
    let mut body = json!({ "message": m.text(), "auto_escape": true });
    if s.str("message_type")? == "group" {
        body["message_type"] = json!("group");
        body["group_id"] = json!(receiver);
    } else {
        body["message_type"] = json!("private");
        body["user_id"] = json!(receiver);
    }
    let mut request = HttpRequest::post(join(&s.base_url("server_url")?, "send_msg"))
        .json(body)
        .accept(classify);
    if let Some(token) = s.get("access_token") {
        request = request.bearer(token);
    }
    Ok(vec![request.into()])
}

/// OneBot 11 answers 200 with `status` `ok`, `async` or `failed` and a `retcode`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => match r.json().get("status").and_then(Value::as_str) {
            Some("ok" | "async") => Verdict::Accepted,
            _ => Verdict::Rejected(r.summary()),
        },
        other => other,
    }
}
