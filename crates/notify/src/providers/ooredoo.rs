use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("ooredoo", "Ooredoo Maldives Bulk SMS", FIELDS, build)
    .docs("https://business.ooredoo.mv/product/bulk-sms-goveli-sms")
    .identity(&["to", "server_url"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("username", "Bulk SMS account user name.").example("bulk-sms-user"),
    Field::secret(
        "access_key",
        "Access key of the account; Vesper Base64-encodes it.",
    ),
    Field::secret("bearer_token", "Bearer token of the account."),
    Field::list(
        "to",
        "Recipient numbers; a 7-digit local number gets the 960 country code.",
    )
    .example("9601234567"),
    Field::url("server_url", "Bulk SMS API endpoint.")
        .default("https://o-papi1-lb01.ooredoo.mv/bulk_sms/v2"),
];

/// The gateway accepts at most this many recipients per request.
const BATCH: usize = 20;

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let recipients: Vec<String> = s
        .list("to")
        .iter()
        .flat_map(|item| item.split_whitespace().map(normalize).collect::<Vec<_>>())
        .filter(|n| !n.is_empty())
        .collect();
    if recipients.is_empty() {
        return Err("setting to needs at least one phone number".into());
    }
    let url = s.str("server_url")?;
    let token = s.str("bearer_token")?;
    let username = s.str("username")?;
    let access_key = base64(s.str("access_key")?);
    let text = m.text();
    Ok(recipients
        .chunks(BATCH)
        .map(|batch| {
            HttpRequest::post(url)
                .bearer(token)
                .form([
                    ("username", username.to_string()),
                    ("access_key", access_key.clone()),
                    ("message", text.clone()),
                    ("batch", batch.join(" ")),
                ])
                .accept(|r| body_code(r, "/response_code", "0"))
                .into()
        })
        .collect())
}

/// Drops `+` and spaces and prefixes a bare 7-digit local number with 960.
fn normalize(number: &str) -> String {
    let number: String = number
        .chars()
        .filter(|c| *c != '+' && !c.is_whitespace())
        .collect();
    if number.len() == 7 && number.bytes().all(|b| b.is_ascii_digit()) {
        format!("960{number}")
    } else {
        number
    }
}
