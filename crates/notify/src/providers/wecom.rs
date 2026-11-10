use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("wecom", "WeCom", FIELDS, build)
    .docs("https://developer.work.weixin.qq.com/document/path/91770")
    .identity(&["bot_key"]);

const FIELDS: &[Field] = &[
    Field::secret(
        "bot_key",
        "Key of the group robot, the `key` parameter of its webhook URL.",
    )
    .example("693a91f6-7xxx-4bc4-97a0-0ec2sifa5aaa"),
    Field::list(
        "mentioned_mobile_list",
        "Mobile numbers of members to mention, or `@all`.",
    )
    .optional()
    .example("13800001111"),
];

/// Text content is limited to 2048 bytes of UTF-8.
const MAX_BYTES: usize = 2048;

fn truncate_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max - '…'.len_utf8();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let mut text = json!({ "content": truncate_bytes(&m.text(), MAX_BYTES) });
    let mobiles = s.list("mentioned_mobile_list");
    if !mobiles.is_empty() {
        text["mentioned_mobile_list"] = json!(mobiles);
    }
    let request = HttpRequest::post("https://qyapi.weixin.qq.com/cgi-bin/webhook/send")
        .query([("key", s.str("bot_key")?)])?
        .json(json!({ "msgtype": "text", "text": text }))
        .accept(classify);
    Ok(vec![request.into()])
}

/// The API answers 200 with `errcode` and `errmsg`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    match r.json().get("errcode").and_then(Value::as_i64) {
        Some(0) => Verdict::Accepted,
        // API frequency out of limit: 20 messages per minute per robot.
        Some(45009) => Verdict::RetryAfter(60),
        Some(-1) => Verdict::Transient(r.summary()),
        _ => Verdict::Rejected(r.summary()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_on_char_boundary_within_byte_limit() {
        // 700 three-byte characters are 2100 bytes: 681 fit with the ellipsis.
        let text = "中".repeat(700);
        let cut = truncate_bytes(&text, MAX_BYTES);
        assert_eq!(cut, format!("{}…", "中".repeat(681)));
        assert!(cut.len() <= MAX_BYTES);
        assert_eq!(truncate_bytes("short", MAX_BYTES), "short");
    }
}
