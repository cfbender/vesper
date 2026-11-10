use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("pushdeer", "PushDeer", FIELDS, build)
    .docs("https://www.pushdeer.com/dev.html")
    .identity(&["server_url", "push_key"]);

const FIELDS: &[Field] = &[
    Field::url(
        "server_url",
        "PushDeer API server, for a self-hosted instance.",
    )
    .default("https://api2.pushdeer.com"),
    Field::secret(
        "push_key",
        "PushKey from the Key tab of the PushDeer app; several keys may be separated by commas.",
    )
    .example("PDU1TTnEZKlRVODU9GkFmtHwaIraV5twUPQbA"),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    // Markdown needs blank lines between entries to keep them apart.
    let desp = format!("{}\n\n{}", m.lines.join("\n\n"), m.link);
    let body = json!({
        "pushkey": s.str("push_key")?,
        "text": m.title,
        "desp": desp,
        "type": "markdown",
    });
    let url = join(&s.base_url("server_url")?, "message/push");
    Ok(vec![HttpRequest::post(url)
        .json(body)
        .accept(classify)
        .into()])
}

/// PushDeer answers `{code, content: {result: [...]}}` or `{code, error}`.
/// Each result is a JSON text (an object on some servers) with
/// `success: "ok"`; an empty list means the key has no registered device.
fn classify(r: &Response) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {}
        other => return other,
    }
    let json = r.json();
    let code_ok = json
        .get("code")
        .is_some_and(|c| c.as_i64() == Some(0) || c == "0");
    if json.get("error").is_some() || !code_ok {
        return Verdict::Rejected(r.summary());
    }
    let Some(first) = json
        .pointer("/content/result")
        .and_then(Value::as_array)
        .and_then(|results| results.first())
    else {
        return Verdict::Rejected("invalid PushDeer key or no device registered".into());
    };
    let result = match first {
        Value::String(text) => serde_json::from_str(text).unwrap_or_default(),
        other => other.clone(),
    };
    if result.get("success").and_then(Value::as_str) == Some("ok") {
        Verdict::Accepted
    } else {
        Verdict::Transient("PushDeer could not deliver the push".into())
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
    fn classifies_push_results() {
        // Sample from doc/api/PushDeerOS.md.
        assert_eq!(
            classify(&resp(
                r#"{"code":0,"content":{"result":["{\"counts\":1,\"logs\":[],\"success\":\"ok\"}"]}}"#
            )),
            Verdict::Accepted
        );
        assert!(matches!(
            classify(&resp(r#"{"code":0,"content":{"result":[]}}"#)),
            Verdict::Rejected(_)
        ));
        assert!(matches!(
            classify(&resp(r#"{"code":80501,"error":"bad key"}"#)),
            Verdict::Rejected(_)
        ));
        assert!(matches!(
            classify(&resp(
                r#"{"code":0,"content":{"result":["{\"success\":\"failed\"}"]}}"#
            )),
            Verdict::Transient(_)
        ));
    }
}
