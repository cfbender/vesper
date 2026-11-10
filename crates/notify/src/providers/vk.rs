use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("vk", "VK", FIELDS, build)
    .docs("https://dev.vk.com/en/method/messages.send")
    .identity(&["peer_id"]);

const FIELDS: &[Field] = &[
    Field::secret(
        "access_token",
        "Community or user access token with the messages permission.",
    ),
    Field::text("api_version", "VK API version to call.").default("5.199"),
    Field::text(
        "peer_id",
        "Destination: a user ID, 2000000000 plus a chat ID for a group chat, or minus a community ID.",
    )
    .example("2000000001"),
    Field::bool(
        "dont_parse_links",
        "Do not turn links in the message into snippets.",
    ),
];

/// `random_id` deduplicates sends for an hour; deriving it from the
/// message makes a retry of the same message idempotent.
fn random_id(peer: &str, m: &Message) -> u32 {
    let digest = sha256(format!("{peer}\n{}\n{}", m.at.to_rfc3339(), m.text()));
    let id = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) & 0x7fff_ffff;
    id.max(1)
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let peer = s.str("peer_id")?;
    let request = HttpRequest::post("https://api.vk.ru/method/messages.send")
        .form([
            ("access_token", s.str("access_token")?.to_string()),
            ("v", s.str("api_version")?.to_string()),
            ("peer_id", peer.to_string()),
            ("message", m.text()),
            (
                "dont_parse_links",
                if s.bool("dont_parse_links") { "1" } else { "0" }.to_string(),
            ),
            ("random_id", random_id(peer, m).to_string()),
        ])
        .accept(classify);
    Ok(vec![request.into()])
}

/// The API answers 200 with `{"response": …}` or `{"error": {error_code,
/// error_msg}}`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let json = r.json();
    if json.get("response").is_some() {
        return Verdict::Accepted;
    }
    let code = json.pointer("/error/error_code").and_then(Value::as_u64);
    let text = json
        .pointer("/error/error_msg")
        .and_then(Value::as_str)
        .map_or_else(|| r.summary(), str::to_string);
    match code {
        // Too many requests per second.
        Some(6) => Verdict::RetryAfter(1),
        // Flood control.
        Some(9) => Verdict::RetryAfter(60),
        // Unknown and internal server errors.
        Some(1 | 10) => Verdict::Transient(format!("API {}: {text}", code.unwrap_or_default())),
        Some(code) => Verdict::Rejected(format!("API {code}: {text}")),
        None => Verdict::Transient(format!("unexpected response: {}", r.summary())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(body: &str) -> Response {
        Response {
            status: 200,
            retry_after: None,
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn classifies_api_errors() {
        assert_eq!(classify(&resp(r#"{"response":12345}"#)), Verdict::Accepted);
        assert_eq!(
            classify(&resp(
                r#"{"error":{"error_code":6,"error_msg":"Too many requests per second"}}"#
            )),
            Verdict::RetryAfter(1)
        );
        assert_eq!(
            classify(&resp(
                r#"{"error":{"error_code":5,"error_msg":"User authorization failed"}}"#
            )),
            Verdict::Rejected("API 5: User authorization failed".into())
        );
        assert!(matches!(
            classify(&resp(
                r#"{"error":{"error_code":10,"error_msg":"Internal server error"}}"#
            )),
            Verdict::Transient(_)
        ));
    }
}
