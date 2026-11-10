use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("smsir", "SMS.ir", FIELDS, build)
    .docs("https://sms.ir/rest-api/")
    .identity(&["mobiles", "template_id"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret(
        "api_key",
        "API key from the SMS.ir panel (Developers, API keys).",
    ),
    Field::list(
        "mobiles",
        "Recipient mobile numbers, such as `9123456789` or `09123456789`.",
    )
    .example("9123456789"),
    Field::int(
        "template_id",
        "ID of the verify (fast send) template defined in the SMS.ir panel.",
    )
    .example("12345"),
    Field::text(
        "parameter_name",
        "Template variable that receives the alert, written `#name#` in the template.",
    )
    .default("uptkumaalert"),
];

/// SMS.ir limits a template variable value to this many characters.
const MAX_VALUE: usize = 20;

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let key = s.str("api_key")?;
    let template = s
        .int("template_id")?
        .ok_or("setting template_id is required")?;
    let value = shorten(&m.title);
    let parameter = s.str("parameter_name")?;
    Ok(s.required_list("mobiles")?
        .iter()
        .map(|mobile| {
            HttpRequest::post("https://api.sms.ir/v1/send/verify")
                .header("X-API-KEY", key)
                .header("Accept", "application/json")
                .json(json!({
                    "mobile": normalize(mobile),
                    "templateId": template,
                    "parameters": [{ "name": parameter, "value": value }],
                }))
                .accept(|r| body_code(r, "/status", "1"))
                .into()
        })
        .collect())
}

/// `09xxxxxxxxx` becomes `9xxxxxxxxx`; other numbers are sent as given.
fn normalize(mobile: &str) -> String {
    match mobile.strip_prefix('0') {
        Some(rest)
            if mobile.len() == 11
                && mobile.starts_with("09")
                && rest.bytes().all(|b| b.is_ascii_digit()) =>
        {
            rest.to_string()
        }
        _ => mobile.to_string(),
    }
}

/// Fits the alert into a template variable: whitespace is dropped first to
/// keep more of the text, then it is cut with `...`.
fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_VALUE {
        return text.to_string();
    }
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.chars().count() <= MAX_VALUE {
        return compact;
    }
    let mut out: String = compact.chars().take(MAX_VALUE - 4).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_template_variable() {
        assert_eq!(shorten("Vesper test message"), "Vesper test message");
        // 24 characters, 21 without spaces: cut to 16 plus "...".
        assert_eq!(shorten("Vesper: 1 new problem(s)"), "Vesper:1newprobl...");
        assert_eq!(shorten("Vesper: 1 resolved"), "Vesper: 1 resolved");
        assert_eq!(shorten("Vesper: 2 resolved now"), "Vesper:2resolvednow");
    }

    #[test]
    fn strips_leading_zero_of_local_numbers() {
        assert_eq!(normalize("09123456789"), "9123456789");
        assert_eq!(normalize("9123456789"), "9123456789");
        assert_eq!(normalize("989123456789"), "989123456789");
    }
}
