use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("dingding", "DingTalk", FIELDS, build)
    .docs("https://open.dingtalk.com/document/robots/custom-robot-access")
    .identity(&["webhook_url"]);

const FIELDS: &[Field] = &[
    Field::secret_url(
        "webhook_url",
        "Webhook URL of the group's custom robot, with its access_token.",
    )
    .example("https://oapi.dingtalk.com/robot/send?access_token=abc123"),
    Field::secret(
        "secret_key",
        "Signing secret (starting with SEC) from the robot's security settings.",
    )
    .example("SECabc123"),
    Field::choice(
        "mentioning",
        &["nobody", "everyone", "specify-mobiles", "specify-users"],
        "Whom the message mentions.",
    )
    .default("nobody"),
    Field::list(
        "mobile_list",
        "Mobile numbers to mention with `specify-mobiles`.",
    )
    .optional()
    .example("13800001111"),
    Field::list("user_list", "User IDs to mention with `specify-users`.")
        .optional()
        .example("user123"),
];

/// Base64 HMAC-SHA256 of `timestamp\nsecret` keyed with the secret.
fn sign(timestamp_ms: i64, secret: &str) -> String {
    base64(hmac_sha256(
        secret.as_bytes(),
        format!("{timestamp_ms}\n{secret}").as_bytes(),
    ))
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let (all, mobiles, users) = match s.str("mentioning")? {
        "everyone" => (true, vec![], vec![]),
        "specify-mobiles" => (false, s.required_list("mobile_list")?, vec![]),
        "specify-users" => (false, vec![], s.required_list("user_list")?),
        _ => (false, vec![], vec![]),
    };
    let mut text = format!(
        "### {}\n\n{}\n\n[Open in Vesper]({})",
        m.title,
        m.lines.join("\n\n"),
        m.link
    );
    let mentions: Vec<String> = mobiles
        .iter()
        .chain(&users)
        .map(|i| format!("@{i}"))
        .collect();
    if !mentions.is_empty() {
        text = format!("{text}\n\n{}", mentions.join(" "));
    }
    let body = json!({
        "msgtype": "markdown",
        "markdown": { "title": m.title, "text": text },
        "at": { "isAtAll": all, "atMobiles": mobiles, "atUserIds": users },
    });
    let timestamp = m.at.timestamp_millis();
    let sign = sign(timestamp, s.str("secret_key")?);
    let request = HttpRequest::post(s.str("webhook_url")?)
        .query([("timestamp", timestamp.to_string()), ("sign", sign)])?
        .json(body)
        .accept(classify);
    Ok(vec![request.into()])
}

/// The robot API answers 200 with `errcode` and `errmsg`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    match r.json().get("errcode").and_then(Value::as_i64) {
        Some(0) => Verdict::Accepted,
        // Sending too fast: limited to 20 messages per minute.
        Some(130101) => Verdict::RetryAfter(60),
        Some(-1) => Verdict::Transient(r.summary()),
        _ => Verdict::Rejected(r.summary()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_timestamp_and_secret() {
        // Computed independently with Python's hmac and base64 modules.
        assert_eq!(
            sign(1_599_360_473_000, "SECdemo"),
            "tGWkA5eQOQNCNPDgphtxIoNLO+/gD8Lcb705KQoRzeY="
        );
    }
}
