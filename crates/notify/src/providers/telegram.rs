use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("telegram", "Telegram", FIELDS, build)
    .docs("https://core.telegram.org/bots/api#sendmessage")
    .identity(&["chat_id", "message_thread_id"]);

const FIELDS: &[Field] = &[
    Field::secret("bot_token", "Bot token from @BotFather.").example("123456:ABC-def_ghi"),
    Field::secret(
        "chat_id",
        "Chat, group or channel ID; message the bot first so it may write to you.",
    )
    .example("-1001234567890"),
    Field::text(
        "message_thread_id",
        "Topic (forum thread) ID in a supergroup.",
    )
    .optional()
    .example("42"),
    Field::bool("silent", "Deliver without sound."),
    Field::bool("protect_content", "Prevent forwarding and saving."),
    Field::url(
        "server_url",
        "Bot API server, for a self-hosted telegram-bot-api.",
    )
    .default("https://api.telegram.org"),
];

fn valid_token(token: &str) -> bool {
    token.len() <= 128
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b':' || b == b'_' || b == b'-')
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let token = s.str("bot_token")?;
    if !valid_token(token) {
        return Err("bot token has an invalid format".into());
    }
    let mut body = json!({
        "chat_id": s.str("chat_id")?,
        "text": m.text(),
        "link_preview_options": { "is_disabled": true },
    });
    if let Some(thread) = s.get("message_thread_id") {
        body["message_thread_id"] = json!(thread);
    }
    if s.bool("silent") {
        body["disable_notification"] = json!(true);
    }
    if s.bool("protect_content") {
        body["protect_content"] = json!(true);
    }
    let url = join(
        &s.base_url("server_url")?,
        &format!("bot{token}/sendMessage"),
    );
    Ok(vec![HttpRequest::post(url)
        .json(body)
        .accept(classify)
        .into()])
}

/// The Bot API reports `ok`, `error_code`, `description` and, when rate
/// limiting, `parameters.retry_after`.
fn classify(r: &Response) -> Verdict {
    let json = r.json();
    if r.status == 200 && json.get("ok").and_then(Value::as_bool) == Some(true) {
        return Verdict::Accepted;
    }
    let code = json
        .get("error_code")
        .and_then(Value::as_u64)
        .map_or(r.status, |c| c as u16);
    if code == 429 || r.status == 429 {
        let retry = json
            .pointer("/parameters/retry_after")
            .and_then(Value::as_u64)
            .or(r.retry_after)
            .unwrap_or(30);
        return Verdict::RetryAfter(retry.max(1));
    }
    let description = json
        .get("description")
        .and_then(Value::as_str)
        .map_or_else(|| format!("HTTP {}", r.status), str::to_string);
    match code {
        400 | 401 | 403 | 404 => Verdict::Rejected(format!("API {code}: {description}")),
        _ => Verdict::Transient(format!("API {code}: {description}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(status: u16, body: &str) -> Response {
        Response {
            status,
            retry_after: None,
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn classifies_bot_api_responses() {
        assert_eq!(
            classify(&resp(200, r#"{"ok":true,"result":{}}"#)),
            Verdict::Accepted
        );
        assert_eq!(
            classify(&resp(
                429,
                r#"{"ok":false,"error_code":429,"parameters":{"retry_after":172800}}"#
            )),
            Verdict::RetryAfter(172_800)
        );
        assert!(matches!(
            classify(&resp(401, r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#)),
            Verdict::Rejected(d) if d == "API 401: Unauthorized"
        ));
        assert!(matches!(
            classify(&resp(502, "<html>")),
            Verdict::Transient(_)
        ));
        // 200 without ok is not acceptance.
        assert!(matches!(classify(&resp(200, "{}")), Verdict::Transient(_)));
    }

    #[test]
    fn token_format_guards_url_path() {
        assert!(valid_token("123456:ABC-def_ghi"));
        assert!(!valid_token("123/../x"));
        assert!(!valid_token("a?b"));
    }
}
