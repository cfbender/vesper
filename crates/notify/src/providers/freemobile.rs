use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("freemobile", "Free Mobile", FIELDS, build)
    .docs("https://mobile.free.fr/account/mes-options")
    .identity(&["user"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text(
        "user",
        "Free Mobile login (identifiant) of the line that receives the SMS.",
    )
    .example("12345678"),
    Field::secret(
        "api_key",
        "Key shown after enabling Mes Options → Notifications par SMS.",
    ),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    Ok(vec![HttpRequest::post(
        "https://smsapi.free-mobile.fr/sendmsg",
    )
    .json(json!({
        "user": s.str("user")?,
        "pass": s.str("api_key")?,
        "msg": m.text(),
    }))
    .accept(classify)
    .into()])
}

/// 402 means too many messages in too short a time.
fn classify(r: &Response) -> Verdict {
    match r.status {
        402 => Verdict::RetryAfter(r.retry_after.unwrap_or(60).max(1)),
        _ => by_status(r),
    }
}
