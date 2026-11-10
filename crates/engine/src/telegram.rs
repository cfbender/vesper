//! Outbound-only Telegram Bot API delivery.

use crate::BoxFuture;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

const API_ORIGIN: &str = "https://api.telegram.org";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// One `sendMessage` call. `Debug` redacts the credentials.
#[derive(Clone)]
pub struct TelegramRequest {
    pub bot_token: String,
    pub chat_id: String,
    pub text: String,
}

impl fmt::Debug for TelegramRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramRequest")
            .field("bot_token", &"[redacted]")
            .field("chat_id", &"[redacted]")
            .field("text", &self.text)
            .finish()
    }
}

/// HTTP response from the Bot API: status code and bounded body bytes.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Sends a Bot API request. `Err` carries a safe description of a network
/// failure whose outcome is ambiguous (Telegram may still have accepted it).
pub trait TelegramTransport: Send + Sync {
    fn send_message<'a>(
        &'a self,
        request: &'a TelegramRequest,
    ) -> BoxFuture<'a, Result<HttpResponse, String>>;
}

/// Production transport: HTTPS POST to the fixed `api.telegram.org` origin,
/// no proxies, no redirects, TLS verification on, bounded time and body.
pub struct HttpsTelegram {
    client: reqwest::Client,
}

impl HttpsTelegram {
    pub fn new() -> anyhow::Result<Self> {
        // Fixed public trust anchors and the ring provider (as vesper-probe
        // uses); no system proxy, CA or crypto-provider state is consulted.
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self { client })
    }
}

fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b':' || b == b'_' || b == b'-')
}

impl TelegramTransport for HttpsTelegram {
    fn send_message<'a>(
        &'a self,
        request: &'a TelegramRequest,
    ) -> BoxFuture<'a, Result<HttpResponse, String>> {
        Box::pin(async move {
            if !valid_token(&request.bot_token) {
                // Surface as an API rejection so it is treated as a configuration error.
                return Ok(HttpResponse {
                    status: 401,
                    body: br#"{"ok":false,"error_code":401,"description":"bot token has an invalid format"}"#.to_vec(),
                });
            }
            let url = format!("{API_ORIGIN}/bot{}/sendMessage", request.bot_token);
            let body = serde_json::json!({
                "chat_id": request.chat_id,
                "text": request.text,
                "link_preview_options": { "is_disabled": true },
            });
            let mut response = self
                .client
                .post(url)
                .json(&body)
                .send()
                .await
                .map_err(|e| network_error(&e))?;
            let status = response.status().as_u16();
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| network_error(&e))? {
                let room = MAX_RESPONSE_BYTES.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
                if room <= chunk.len() {
                    break;
                }
            }
            Ok(HttpResponse {
                status,
                body: bytes,
            })
        })
    }
}

/// Describes a reqwest error without its URL (which contains the bot token).
fn network_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "request to Telegram timed out".into()
    } else if e.is_connect() {
        "could not connect to Telegram".into()
    } else if e.is_body() || e.is_decode() {
        "connection to Telegram failed while reading the response".into()
    } else {
        "request to Telegram failed".into()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Accepted,
    /// Telegram asked to wait this many seconds.
    RetryAfter(u64),
    /// Authentication/permission/configuration rejection; do not retry blindly.
    Rejected(String),
    /// Server or network failure; retry with backoff.
    Transient(String),
}

pub(crate) fn classify(result: Result<HttpResponse, String>, secrets: &[&str]) -> Outcome {
    let response = match result {
        Ok(r) => r,
        Err(e) => return Outcome::Transient(scrub(&e, secrets)),
    };
    let json: serde_json::Value = serde_json::from_slice(&response.body).unwrap_or_default();
    let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let code = json
        .get("error_code")
        .and_then(|v| v.as_u64())
        .map(|c| c as u16)
        .unwrap_or(response.status);
    let description = json
        .get("description")
        .and_then(|v| v.as_str())
        .map(|d| scrub(d, secrets))
        .unwrap_or_else(|| format!("HTTP {}", response.status));
    if response.status == 200 && ok {
        return Outcome::Accepted;
    }
    if code == 429 || response.status == 429 {
        let retry = json
            .pointer("/parameters/retry_after")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);
        return Outcome::RetryAfter(retry.clamp(1, 86_400));
    }
    match code {
        400 | 401 | 403 | 404 => Outcome::Rejected(format!("Telegram API {code}: {description}")),
        _ => Outcome::Transient(format!("Telegram API {code}: {description}")),
    }
}

/// Bounds text, strips control characters and removes credential values.
pub(crate) fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut out: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
    for secret in secrets {
        if secret.len() >= 4 {
            out = out.replace(secret, "[redacted]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(status: u16, body: &str) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status,
            body: body.as_bytes().to_vec(),
        })
    }

    #[test]
    fn classifies_bot_api_responses() {
        assert_eq!(
            classify(resp(200, r#"{"ok":true,"result":{}}"#), &[]),
            Outcome::Accepted
        );
        assert_eq!(
            classify(
                resp(
                    429,
                    r#"{"ok":false,"error_code":429,"description":"Too Many Requests: retry after 17","parameters":{"retry_after":17}}"#
                ),
                &[]
            ),
            Outcome::RetryAfter(17)
        );
        assert!(matches!(
            classify(resp(401, r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#), &[]),
            Outcome::Rejected(d) if d.contains("Unauthorized")
        ));
        assert!(matches!(
            classify(resp(502, "<html>"), &[]),
            Outcome::Transient(_)
        ));
        assert!(matches!(
            classify(resp(400, r#"{"ok":false,"error_code":400,"description":"chat 12345:SECRET not found"}"#), &["12345:SECRET"]),
            Outcome::Rejected(d) if !d.contains("SECRET")
        ));
    }

    #[test]
    fn token_format_guards_url_path() {
        assert!(valid_token("123456:ABC-def_ghi"));
        assert!(!valid_token("123/../x"));
        assert!(!valid_token("a?b"));
        assert!(!valid_token(""));
    }
}
