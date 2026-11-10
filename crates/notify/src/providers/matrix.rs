use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("matrix", "Matrix", FIELDS, build)
    .docs("https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3roomsroomidsendeventtypetxnid")
    .identity(&["homeserver_url", "room_id"]);

const FIELDS: &[Field] = &[
    Field::url("homeserver_url", "Client-server API URL of the homeserver.")
        .example("https://matrix.example.org"),
    Field::text(
        "room_id",
        "Internal room ID (Room settings → Advanced), not a room alias; the account must have joined it.",
    )
    .example("!AbCdEfGhIjKlMnOp:example.org"),
    Field::secret("access_token", "Access token of the sending account."),
];

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let text = m.text();
    // A transaction ID derived from the message makes a retried send
    // idempotent: the homeserver drops the duplicate.
    let txn = hex(sha256(format!(
        "{}\n{}\n{text}",
        m.dedup_key(),
        m.at.to_rfc3339()
    )));
    let url = join(
        &s.base_url("homeserver_url")?,
        &format!(
            "_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            encode(s.str("room_id")?),
            &txn[..32]
        ),
    );
    let body = json!({ "msgtype": "m.text", "body": text });
    Ok(vec![HttpRequest::put(url)
        .bearer(s.str("access_token")?)
        .json(body)
        .accept(classify)
        .into()])
}

/// Homeservers report rate limits as `M_LIMIT_EXCEEDED` with
/// `retry_after_ms` in the body, not always with a `Retry-After` header.
fn classify(r: &Response) -> Verdict {
    if r.status == 429 && r.retry_after.is_none() {
        if let Some(ms) = r.json().get("retry_after_ms").and_then(Value::as_u64) {
            return Verdict::RetryAfter(ms.div_ceil(1000).max(1));
        }
    }
    by_status(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_reads_retry_after_ms() {
        let r = Response {
            status: 429,
            retry_after: None,
            body: br#"{"errcode":"M_LIMIT_EXCEEDED","error":"Too many requests","retry_after_ms":2001}"#
                .to_vec(),
        };
        assert_eq!(classify(&r), Verdict::RetryAfter(3));
    }
}
