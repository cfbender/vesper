use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("serverchan", "ServerChan", FIELDS, build)
    .docs("https://doc.sc3.ft07.com/en/serverchan3/server/api")
    .identity(&["send_key"]);

const FIELDS: &[Field] = &[Field::secret(
    "send_key",
    "SendKey of ServerChan Turbo (SCT…) or ServerChan³ (sctp…); the key decides the endpoint.",
)
.example("SCT123456TAbCdEfGhIjKlMnOp")];

/// ServerChan³ keys look like `sctp{uid}t…` and are sent to
/// `{uid}.push.ft07.com`; other (Turbo) keys go to `sctapi.ftqq.com`.
fn endpoint(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("sctp") {
        let uid: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !uid.is_empty() && rest[uid.len()..].starts_with('t') {
            return format!("https://{uid}.push.ft07.com/send/{}.send", encode(key));
        }
    }
    format!("https://sctapi.ftqq.com/{}.send", encode(key))
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    // The title must be one line; the body is Markdown.
    let title = truncate(&m.title.replace('\n', " "), 32);
    let desp = format!("{}\n\n{}", m.lines.join("\n\n"), m.link);
    Ok(vec![HttpRequest::post(endpoint(s.str("send_key")?))
        .json(json!({ "title": title, "desp": desp }))
        .accept(|r| body_code(r, "/code", "0"))
        .into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_selects_endpoint() {
        // Per the ServerChan³ API docs: uid from /^sctp(\d+)t/.
        assert_eq!(
            endpoint("sctp123tAbC"),
            "https://123.push.ft07.com/send/sctp123tAbC.send"
        );
        assert_eq!(
            endpoint("SCT9999TxYz"),
            "https://sctapi.ftqq.com/SCT9999TxYz.send"
        );
        // Without the `t` after the digits it is not a ServerChan³ key.
        assert_eq!(endpoint("sctp12x"), "https://sctapi.ftqq.com/sctp12x.send");
    }
}
