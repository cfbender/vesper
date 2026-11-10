use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("amootsms", "Amoot SMS", FIELDS, build)
    .docs("https://portal.amootsms.com/documentation")
    .identity(&["mobiles", "line_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::secret("api_token", "API token from the Amoot SMS portal."),
    Field::list("mobiles", "Recipient mobile numbers.").example("9123456789"),
    Field::text(
        "line_number",
        "Sender line number, or `public` for the shared line.",
    )
    .default("public"),
    Field::bool(
        "use_pattern",
        "Send through an approved pattern instead of free text.",
    ),
    Field::int(
        "pattern_code_id",
        "Pattern code ID from the portal, required with use_pattern.",
    )
    .optional()
    .example("1234"),
    Field::bool(
        "use_own_line",
        "Send the pattern from line_number instead of the shared pattern line.",
    ),
];

const API: &str = "https://portal.amootsms.com/rest";

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let token = s.str("api_token")?;
    let mobiles = s.required_list("mobiles")?;
    let line = s.str("line_number")?;
    let text = m.text();

    if !s.bool("use_pattern") {
        // The portal expects the send time in Tehran time (UTC+03:30).
        let tehran = m.at + chrono::Duration::minutes(210);
        let request = HttpRequest::post(format!("{API}/SendSimple"))
            .header("Authorization", token)
            .form([
                (
                    "SendDateTime",
                    tehran.format("%Y-%m-%d %H:%M:%S").to_string(),
                ),
                ("SMSMessageText", text),
                ("LineNumber", line.to_string()),
                ("Mobiles", mobiles.join(",")),
            ])
            .accept(accept_simple);
        return Ok(vec![request.into()]);
    }

    let pattern = s
        .int("pattern_code_id")?
        .ok_or("setting pattern_code_id is required with use_pattern")?;
    let own_line = s.bool("use_own_line");
    Ok(mobiles
        .iter()
        .map(|mobile| {
            let request = if own_line {
                HttpRequest::post(format!("{API}/SendWithPatternOWN"))
                    .bearer(token)
                    .json(json!({
                        "Token": token,
                        "LineNumber": line,
                        "Mobile": mobile,
                        "PatternCodeID": pattern,
                        "PatternValues": text,
                    }))
            } else {
                HttpRequest::post(format!("{API}/SendWithPattern"))
                    .header("Authorization", token)
                    .form([
                        ("Token", token.to_string()),
                        ("Mobile", mobile.clone()),
                        ("PatternCodeID", pattern.to_string()),
                        ("PatternValues", text.clone()),
                    ])
            };
            request.accept(accept_pattern).into()
        })
        .collect())
}

/// Every call answers `{Status, ErrorMessage, …}`; `Status` is `Success`
/// or an error name such as `Token_Invalid`.
fn accept_pattern(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {
            let json = r.json();
            if !json.is_object() {
                return Verdict::Transient(format!("unexpected response: {}", r.summary()));
            }
            match json.get("Status").and_then(Value::as_str) {
                None | Some("Success") => Verdict::Accepted,
                Some(status) => {
                    let detail = json
                        .get("ErrorMessage")
                        .and_then(Value::as_str)
                        .filter(|e| !e.is_empty())
                        .map_or(String::new(), |e| format!(": {e}"));
                    // `ServerError` also reports malformed parameters, so
                    // every error status is treated as a rejection.
                    Verdict::Rejected(format!("API error {status}{detail}"))
                }
            }
        }
        other => other,
    }
}

/// `SendSimple` additionally reports one `Data` entry per recipient whose
/// `Status` is `1` (or `Success`) when it was sent.
fn accept_simple(r: &Response) -> Verdict {
    match accept_pattern(r) {
        Verdict::Accepted => {
            let json = r.json();
            let Some(items) = json
                .get("Data")
                .and_then(Value::as_array)
                .filter(|d| !d.is_empty())
            else {
                return Verdict::Transient(format!("unexpected response: {}", r.summary()));
            };
            let sent =
                |v: &Value| v.as_i64() == Some(1) || matches!(v.as_str(), Some("1" | "Success"));
            match items
                .iter()
                .map(|i| i.get("Status").cloned().unwrap_or_default())
                .find(|s| !sent(s))
            {
                None => Verdict::Accepted,
                Some(status) => Verdict::Rejected(format!("API error status {status}")),
            }
        }
        other => other,
    }
}
