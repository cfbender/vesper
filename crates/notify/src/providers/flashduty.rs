use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("flashduty", "Flashduty", FIELDS, build)
    .docs("https://docs.flashduty.com/en/on-call/integration/alert-integration/alert-sources/standard-alert")
    .identity(&["integration_key"])
    .per_incident();

const FIELDS: &[Field] = &[
    Field::secret(
        "integration_key",
        "Integration key of a standard alert integration, or its full push URL.",
    ),
    Field::choice(
        "severity",
        &["Info", "Warning", "Critical"],
        "Severity of problems and tests.",
    )
    .default("Warning"),
];

const PUSH_URL: &str = "https://api.flashcat.cloud/event/push/alert/standard";

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let key = s.str("integration_key")?;
    let request = if key.starts_with("https://") || key.starts_with("http://") {
        HttpRequest::post(key)
    } else {
        HttpRequest::post(PUSH_URL).query([("integration_key", key)])?
    };
    let status = match m.status {
        Status::Up => "Ok",
        _ => s.str("severity")?,
    };
    let mut labels = json!({ "source": "Vesper" });
    if let Some(i) = &m.incident {
        labels["resource"] = json!(i.service_id);
        labels["check"] = json!(i.service_name);
    }
    let body = json!({
        "title_rule": truncate(&m.title, 512),
        "description": truncate(&m.body(), 2048),
        "event_status": status,
        "alert_key": m.dedup_key(),
        "labels": labels,
    });
    Ok(vec![request.json(body).accept(classify).into()])
}

/// Errors carry an `error` object with `code` and `message`, which may come
/// with a successful status.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted if r.json().get("error").is_some_and(|e| !e.is_null()) => {
            Verdict::Rejected(r.summary())
        }
        other => other,
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
    fn rejects_errors_in_successful_responses() {
        // Response shapes from the standard alert event documentation.
        assert_eq!(
            classify(&resp(
                200,
                r#"{"request_id":"0ace00116215ab4ca0ec5244b8fc54b0","data":{"alert_key":"k"}}"#
            )),
            Verdict::Accepted
        );
        assert!(matches!(
            classify(&resp(
                200,
                r#"{"request_id":"r","error":{"code":"InvalidParameter","message":"integration_key is not a valid one"}}"#
            )),
            Verdict::Rejected(d) if d == "HTTP 200: integration_key is not a valid one"
        ));
        assert!(matches!(classify(&resp(500, "{}")), Verdict::Transient(_)));
    }
}
