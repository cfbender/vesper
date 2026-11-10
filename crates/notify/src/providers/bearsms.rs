use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("bearsms", "BearSMS", FIELDS, build)
    .docs("https://www.bearsms.com/1-q-a/")
    .identity(&["phone_number"])
    .max_chars(600);

const FIELDS: &[Field] = &[
    Field::text("username", "BearSMS API username."),
    Field::secret(
        "hash_key",
        "Webservices token (hash key) from the BearSMS preferences.",
    ),
    Field::text(
        "phone_number",
        "Recipient number in international format without a plus sign.",
    )
    .example("972501234567"),
    Field::text("sender_id", "Sender name or number of up to 11 characters.")
        .optional()
        .example("Vesper"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let text = m.text();
    let mut query = vec![
        ("app", "ws".to_string()),
        ("u", s.str("username")?.to_string()),
        ("h", s.str("hash_key")?.to_string()),
        ("op", "pv".to_string()),
        ("to", s.str("phone_number")?.to_string()),
    ];
    if let Some(sender) = s.get("sender_id") {
        query.push(("from", sender.to_string()));
    }
    // Text outside GSM (such as Hebrew) must be flagged as Unicode.
    if !text.is_ascii() {
        query.push(("unicode", "1".to_string()));
    }
    query.push(("msg", text));
    Ok(vec![HttpRequest::get("https://app.bearsms.com/index.php")
        .query(query)?
        .accept(classify)
        .into()])
}

/// playSMS webservices answer 200 either way: failure is
/// `{"status":"ERR","error":"100","error_string":"authentication failed"}`,
/// success `{"data":[{"status":"OK","error":"0",…}],"error_string":null}`.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let json = r.json();
    let error = json
        .get("error_string")
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty());
    let sent = json.get("data").and_then(Value::as_array).is_some_and(|d| {
        d.iter()
            .any(|m| m.get("status").and_then(Value::as_str) == Some("OK"))
    });
    match (json.get("status").and_then(Value::as_str), error) {
        (Some("ERR"), _) | (_, Some(_)) => {
            Verdict::Rejected(error.map_or_else(|| r.summary(), str::to_string))
        }
        _ if sent => Verdict::Accepted,
        _ => Verdict::Rejected(r.summary()),
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
    fn judges_playsms_bodies() {
        assert_eq!(
            classify(&resp(
                r#"{"data":[{"status":"OK","error":"0","smslog_id":"30","to":"972501234567"}],"error_string":null}"#
            )),
            Verdict::Accepted
        );
        assert_eq!(
            classify(&resp(
                r#"{"status":"ERR","error":"100","error_string":"authentication failed"}"#
            )),
            Verdict::Rejected("authentication failed".into())
        );
        // A per-recipient failure without an overall error is not delivery.
        assert!(matches!(
            classify(&resp(
                r#"{"data":[{"status":"ERR","error":"200"}],"error_string":null}"#
            )),
            Verdict::Rejected(_)
        ));
    }
}
