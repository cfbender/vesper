use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("milky", "Milky", FIELDS, build)
    .docs("https://milky.ntqqrev.org/api/message")
    .identity(&["server_url", "message_type", "receiver_id"]);

const FIELDS: &[Field] = &[
    Field::url(
        "server_url",
        "HTTP address of the Milky implementation, without the `/api` path.",
    )
    .example("http://127.0.0.1:3000"),
    Field::secret(
        "access_token",
        "Access token configured in the Milky implementation.",
    )
    .optional(),
    Field::choice(
        "message_type",
        &["private", "group"],
        "Send to a friend (`private`) or a group (`group`).",
    )
    .default("private"),
    Field::text(
        "receiver_id",
        "QQ number of the friend, or the group number.",
    )
    .example("123456789"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let receiver: i64 = s
        .str("receiver_id")?
        .parse()
        .map_err(|_| "setting receiver_id must be a number".to_string())?;
    let mut body = json!({
        "message": [{ "type": "text", "data": { "text": m.text() } }],
    });
    let endpoint = if s.str("message_type")? == "group" {
        body["group_id"] = json!(receiver);
        "api/send_group_message"
    } else {
        body["user_id"] = json!(receiver);
        "api/send_private_message"
    };
    let mut request = HttpRequest::post(join(&s.base_url("server_url")?, endpoint))
        .json(body)
        .accept(classify);
    if let Some(token) = s.get("access_token") {
        request = request.bearer(token);
    }
    Ok(vec![request.into()])
}

/// Milky answers 200 with `retcode` 0 on success, or `status: failed`,
/// a negative `retcode` and a `message`.
fn classify(r: &Response) -> Verdict {
    body_code(r, "/retcode", "0")
}
