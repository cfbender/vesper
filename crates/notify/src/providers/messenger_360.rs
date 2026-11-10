use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("360messenger", "360messenger", FIELDS, build)
    .docs("https://developer.360messenger.com/")
    .identity(&["recipient_type", "recipients"]);

const FIELDS: &[Field] = &[
    Field::secret("auth_token", "API key from the 360messenger dashboard."),
    Field::choice(
        "recipient_type",
        &["phone", "group"],
        "Whether recipients are WhatsApp phone numbers or group IDs.",
    )
    .default("phone"),
    Field::list(
        "recipients",
        "Phone numbers with country code and no `+`, or WhatsApp group IDs.",
    )
    .example("447488888888"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let token = s.str("auth_token")?;
    let group = s.str("recipient_type")? == "group";
    let text = m.text();
    Ok(s.required_list("recipients")?
        .into_iter()
        .map(|recipient| {
            let (url, body) = if group {
                (
                    "https://api.360messenger.com/v2/sendGroup",
                    json!({ "groupId": recipient, "text": text }),
                )
            } else {
                (
                    "https://api.360messenger.com/v2/sendMessage",
                    json!({ "phonenumber": recipient, "text": text }),
                )
            };
            HttpRequest::post(url)
                .bearer(token)
                .header("Accept", "application/json")
                .json(body)
                .into()
        })
        .collect())
}
