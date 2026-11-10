use super::prelude::*;

pub const PROVIDER: Provider = Provider::new("nextcloud_talk", "Nextcloud Talk", FIELDS, build)
    .docs("https://nextcloud-talk.readthedocs.io/en/latest/bots/#sending-a-chat-message")
    .identity(&["server_url", "conversation_token"]);

const FIELDS: &[Field] = &[
    Field::url("server_url", "Address of the Nextcloud server.")
        .example("https://cloud.example.com"),
    Field::text(
        "conversation_token",
        "Token of the conversation the bot is set up in, from its URL (`/call/<token>`).",
    )
    .example("a1b2c3d4"),
    Field::secret(
        "bot_secret",
        "Secret given to `occ talk:bot:install` when installing the bot.",
    ),
    Field::bool("send_silent_up", "Send recoveries without a notification."),
    Field::bool("send_silent_down", "Send problems without a notification."),
];

/// Hex HMAC-SHA256 over the random value followed by the message.
fn sign(secret: &str, random: &str, message: &str) -> String {
    hex(hmac_sha256(
        secret.as_bytes(),
        format!("{random}{message}").as_bytes(),
    ))
}

fn build(s: &Settings, m: &Message) -> Result<Vec<Request>, String> {
    let message = m.text();
    // 64 hex characters derived from the message; the signature, not the
    // random value, authenticates the request.
    let random = hex(sha256(format!("{}\n{message}", m.at.to_rfc3339())));
    let signature = sign(s.str("bot_secret")?, &random, &message);
    let silent = match m.status {
        Status::Up => s.bool("send_silent_up"),
        Status::Down => s.bool("send_silent_down"),
        Status::Test => false,
    };
    let url = join(
        &s.base_url("server_url")?,
        &format!(
            "ocs/v2.php/apps/spreed/api/v1/bot/{}/message",
            encode(s.str("conversation_token")?)
        ),
    );
    Ok(vec![HttpRequest::post(url)
        .header("OCS-APIRequest", "true")
        .header("X-Nextcloud-Talk-Bot-Random", random)
        .header("X-Nextcloud-Talk-Bot-Signature", signature)
        .json(json!({ "message": message, "silent": silent }))
        .into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_openssl() {
        // printf '%s' 'r4nd0mHello, Talk!' | openssl dgst -sha256 -hmac 'bot-secret-123'
        assert_eq!(
            sign("bot-secret-123", "r4nd0m", "Hello, Talk!"),
            "b5e5d41bd47c8c1e98f087ff177b20257a8883bfc415c7ea9c650907233c7091"
        );
    }
}
