use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("bale", "Bale", FIELDS, build)
    .docs("https://docs.bale.ai/")
    .identity(&["chat_id"]);

const FIELDS: &[Field] = &[
    Field::secret("bot_token", "Bot token from @BotFather on Bale.")
        .example("123456789:ABCdef_ghi"),
    Field::text(
        "chat_id",
        "Chat, group or channel ID; message the bot first so it may write to you.",
    )
    .example("1234567890"),
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
    let body = json!({ "chat_id": s.str("chat_id")?, "text": m.text() });
    let url = format!("https://tapi.bale.ai/bot{token}/sendMessage");
    Ok(vec![HttpRequest::post(url)
        .json(body)
        .accept(classify)
        .into()])
}

/// The Bot API mirrors Telegram's: `ok`, `error_code`, `description` and,
/// when rate limiting (429, or Bale's 420 Flood), `parameters.retry_after`.
fn classify(r: &Response) -> Verdict {
    let json = r.json();
    if r.success() && json.get("ok").and_then(Value::as_bool) == Some(true) {
        return Verdict::Accepted;
    }
    let code = json
        .get("error_code")
        .and_then(Value::as_u64)
        .map_or(r.status, |c| c as u16);
    if matches!(code, 420 | 429) || matches!(r.status, 420 | 429) {
        let retry = json
            .pointer("/parameters/retry_after")
            .and_then(Value::as_u64)
            .or(r.retry_after)
            .unwrap_or(30);
        return Verdict::RetryAfter(retry.max(1));
    }
    match code {
        400..=499 if code != 408 => Verdict::Rejected(r.summary()),
        _ => Verdict::Transient(r.summary()),
    }
}
