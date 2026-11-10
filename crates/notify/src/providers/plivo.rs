use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("plivo", "Plivo", FIELDS, build)
    .docs("https://www.plivo.com/docs/messaging/api/message")
    .identity(&["to_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("auth_id", "Auth ID from the Plivo console.").example("MAXXXXXXXXXXXXXXXXXX"),
    Field::secret("auth_token", "Auth token from the Plivo console."),
    Field::text(
        "from_number",
        "Plivo number in E.164 format, or a sender ID, to send or call from.",
    )
    .example("+15551234567"),
    Field::text("to_number", "Recipient phone number in E.164 format.").example("+15559876543"),
    Field::choice(
        "message_type",
        &["sms", "call"],
        "`sms` sends a text message; `call` places a voice call.",
    )
    .default("sms"),
    Field::url(
        "answer_url",
        "Plivo XML answer URL for voice calls; the message is added as its `message` query parameter.",
    )
    .optional()
    .example("https://example.com/answer.xml"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let auth_id = s.str("auth_id")?;
    let from = s.str("from_number")?;
    let to = s.str("to_number")?;
    let base = format!("https://api.plivo.com/v1/Account/{}", encode(auth_id));
    let request = match s.str("message_type")? {
        "call" => {
            let answer = s
                .get("answer_url")
                .ok_or("setting answer_url is required for voice calls")?;
            let mut url =
                url::Url::parse(answer).map_err(|_| "setting answer_url is not a valid URL")?;
            let kept: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(k, _)| k != "message")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(kept)
                .append_pair("message", &m.text());
            HttpRequest::post(format!("{base}/Call/")).json(json!({
                "from": from,
                "to": to,
                "answer_url": url.to_string(),
                "answer_method": "GET",
            }))
        }
        _ => HttpRequest::post(format!("{base}/Message/")).json(json!({
            "src": from,
            "dst": to,
            "text": m.text(),
        })),
    };
    Ok(vec![request.basic(auth_id, s.str("auth_token")?).into()])
}
