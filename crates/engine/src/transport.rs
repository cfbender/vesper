//! Outbound notification delivery over HTTP(S) and SMTP.
//!
//! Every destination host is resolved here and each address is checked with
//! [`vesper_probe::permit_notification_address`]: public addresses are
//! reachable, private networks only when `allowed_cidrs` lists them, and
//! loopback, link-local and metadata addresses never. Connections go to the
//! checked addresses only. No proxies, no redirects, TLS always verified
//! against public roots plus the optional private CA, bounded time and body.

use crate::BoxFuture;
use rustls::pki_types::{pem::PemObject, CertificateDer};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use vesper_notify::{HttpRequest, MailRequest, MailSecurity, RequestBody, Response, Verdict};

const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_RESOLVED_ADDRESSES: usize = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

/// The destination policy for one delivery.
#[derive(Debug, Clone, Default)]
pub struct Network {
    pub allowed_cidrs: Vec<String>,
    /// PEM of the configured private CA, trusted in addition to public roots.
    pub private_ca: Option<String>,
}

/// Sends one request. HTTP returns the response, or the verdict for a
/// request that never got one: `Rejected` when policy or the URL forbids it,
/// `Transient` for network failures whose outcome is ambiguous. Error texts
/// never contain URLs, which may embed credentials.
pub trait NotifyTransport: Send + Sync {
    fn http<'a>(
        &'a self,
        request: &'a HttpRequest,
        network: &'a Network,
    ) -> BoxFuture<'a, Result<Response, Verdict>>;

    fn mail<'a>(&'a self, request: &'a MailRequest, network: &'a Network)
        -> BoxFuture<'a, Verdict>;
}

/// Production transport.
pub struct NetworkTransport;

/// Resolves `host` and keeps the addresses notifications may reach.
async fn permitted_addresses(
    host: &str,
    port: u16,
    network: &Network,
) -> Result<Vec<IpAddr>, Verdict> {
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let candidates: Vec<IpAddr> = match unbracketed.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| Verdict::Transient("DNS lookup timed out".into()))?
            .map_err(|_| Verdict::Transient(format!("could not resolve {host}")))?
            .take(MAX_RESOLVED_ADDRESSES)
            .map(|a| a.ip())
            .collect(),
    };
    let mut permitted = vec![];
    let mut denied = None;
    for ip in candidates {
        match vesper_probe::permit_notification_address(ip, &network.allowed_cidrs) {
            Ok(ip) if !permitted.contains(&ip) => permitted.push(ip),
            Ok(_) => {}
            Err(reason) => denied = Some(reason),
        }
    }
    if permitted.is_empty() {
        return Err(match denied {
            Some(reason) => Verdict::Rejected(format!("destination denied: {reason}")),
            None => Verdict::Transient(format!("{host} has no addresses")),
        });
    }
    Ok(permitted)
}

fn tls_config(network: &Network) -> Result<rustls::ClientConfig, Verdict> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(pem) = &network.private_ca {
        for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
            let cert =
                cert.map_err(|_| Verdict::Rejected("private CA file is not valid PEM".into()))?;
            roots.add(cert).map_err(|_| {
                Verdict::Rejected("private CA file contains an invalid certificate".into())
            })?;
        }
    }
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| Verdict::Transient("TLS configuration failed".into()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(tls)
}

/// Refuses every lookup, so a host without a checked override never resolves.
struct NoDns;
impl reqwest::dns::Resolve for NoDns {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async { Err("unchecked DNS lookups are disabled".into()) })
    }
}

/// Describes a reqwest error without its URL.
fn network_error(e: &reqwest::Error) -> Verdict {
    Verdict::Transient(
        if e.is_timeout() {
            "request timed out"
        } else if e.is_connect() {
            "could not connect"
        } else if e.is_body() || e.is_decode() {
            "connection failed while reading the response"
        } else {
            "request failed"
        }
        .into(),
    )
}

async fn send_http(request: &HttpRequest, network: &Network) -> Result<Response, Verdict> {
    let url = url::Url::parse(&request.url).map_err(|_| Verdict::Rejected("invalid URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Verdict::Rejected(
            "URL must be http(s) without credentials".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Verdict::Rejected("URL has no host".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = permitted_addresses(host, port, network).await?;
    let mut client = reqwest::Client::builder()
        .tls_backend_preconfigured(tls_config(network)?)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .dns_resolver(Arc::new(NoDns));
    if host.parse::<IpAddr>().is_err() && !host.starts_with('[') {
        let addrs: Vec<SocketAddr> = addresses
            .iter()
            .map(|ip| SocketAddr::new(*ip, port))
            .collect();
        client = client.resolve_to_addrs(host, &addrs);
    }
    let client = client
        .build()
        .map_err(|_| Verdict::Transient("HTTP client setup failed".into()))?;
    let method = reqwest::Method::from_bytes(request.method.as_str().as_bytes())
        .expect("static method name");
    let mut builder = client.request(method, url);
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder = match &request.body {
        RequestBody::Empty => builder,
        RequestBody::Json(value) => builder.json(value),
        RequestBody::Form(pairs) => builder
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(
                url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(pairs)
                    .finish(),
            ),
        RequestBody::Text { content_type, text } => builder
            .header("Content-Type", content_type.as_str())
            .body(text.clone()),
    };
    let mut response = builder.send().await.map_err(|e| network_error(&e))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| network_error(&e))? {
        let room = MAX_RESPONSE_BYTES.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if room <= chunk.len() {
            break;
        }
    }
    Ok(Response {
        status,
        retry_after,
        body,
    })
}

async fn send_mail(request: &MailRequest, network: &Network) -> Verdict {
    use lettre::message::{header::ContentType, Mailbox};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::transport::smtp::client::{Certificate, Tls, TlsParameters};
    use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

    let mailbox = |field: &str, text: &str| {
        text.parse::<Mailbox>()
            .map_err(|_| Verdict::Rejected(format!("invalid email address in {field}")))
    };
    let mut builder = match mailbox("from", &request.from) {
        Ok(from) => lettre::Message::builder()
            .from(from)
            .subject(request.subject.clone()),
        Err(v) => return v,
    };
    for (field, list) in [
        ("to", &request.to),
        ("cc", &request.cc),
        ("bcc", &request.bcc),
    ] {
        for address in list {
            match mailbox(field, address) {
                Ok(m) => {
                    builder = match field {
                        "to" => builder.to(m),
                        "cc" => builder.cc(m),
                        _ => builder.bcc(m),
                    }
                }
                Err(v) => return v,
            }
        }
    }
    let message = match builder
        .header(ContentType::TEXT_PLAIN)
        .body(request.body.clone())
    {
        Ok(m) => m,
        Err(_) => return Verdict::Rejected("could not compose the email".into()),
    };
    let addresses = match permitted_addresses(&request.host, request.port, network).await {
        Ok(a) => a,
        Err(v) => return v,
    };
    let mut tls = TlsParameters::builder(request.host.clone());
    if let Some(pem) = &network.private_ca {
        match Certificate::from_pem(pem.as_bytes()) {
            Ok(cert) => tls = tls.add_root_certificate(cert),
            Err(_) => return Verdict::Rejected("private CA file is not valid PEM".into()),
        }
    }
    let tls = match tls.build_rustls() {
        Ok(t) => t,
        Err(_) => return Verdict::Transient("TLS configuration failed".into()),
    };
    let tls = match request.security {
        MailSecurity::Tls => Tls::Wrapper(tls),
        MailSecurity::StartTls => Tls::Required(tls),
        MailSecurity::None => Tls::None,
    };
    // Connect to the checked address; TLS still verifies the host name.
    let mut transport =
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(addresses[0].to_string())
            .port(request.port)
            .tls(tls)
            .timeout(Some(TOTAL_TIMEOUT));
    if let Some(user) = &request.username {
        transport = transport.credentials(Credentials::new(
            user.clone(),
            request.password.clone().unwrap_or_default(),
        ));
    }
    let transport = transport.build();
    match tokio::time::timeout(TOTAL_TIMEOUT, transport.send(message)).await {
        Err(_) => Verdict::Transient("SMTP delivery timed out".into()),
        Ok(Ok(_)) => Verdict::Accepted,
        Ok(Err(e)) if e.is_permanent() => Verdict::Rejected(format!("SMTP {e}")),
        // Transient replies, connection and TLS failures: retry with backoff.
        Ok(Err(e)) => Verdict::Transient(format!("SMTP {e}")),
    }
}

impl NotifyTransport for NetworkTransport {
    fn http<'a>(
        &'a self,
        request: &'a HttpRequest,
        network: &'a Network,
    ) -> BoxFuture<'a, Result<Response, Verdict>> {
        Box::pin(send_http(request, network))
    }

    fn mail<'a>(
        &'a self,
        request: &'a MailRequest,
        network: &'a Network,
    ) -> BoxFuture<'a, Verdict> {
        Box::pin(send_mail(request, network))
    }
}

/// Bounds text, strips control characters and removes credential values.
pub(crate) fn scrub(text: &str, secrets: &[String]) -> String {
    // Redact first so truncation can never cut a secret into a leaked prefix.
    let mut out = text.to_string();
    for secret in secrets {
        if secret.len() >= 4 {
            out = out.replace(secret.as_str(), "[redacted]");
        }
    }
    out.chars().filter(|c| !c.is_control()).take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_before_bounding() {
        let secret = "123456:SECRETTOKENVALUE".to_string();
        // The secret straddles the 200-character bound.
        let text = format!("{}{secret} tail", "x".repeat(190));
        let out = scrub(&text, &[secret]);
        assert!(!out.contains("123456"), "{out}");
        assert_eq!(out.chars().count(), 200);
    }

    #[tokio::test]
    async fn private_and_local_destinations_are_refused_before_connecting() {
        let network = Network {
            allowed_cidrs: vec!["10.203.44.0/24".into()],
            private_ca: None,
        };
        for url in [
            "http://127.0.0.1:9/x",
            "http://192.168.1.5/x",
            "http://[::1]/x",
            "http://169.254.169.254/latest",
        ] {
            let request = HttpRequest::post(url);
            match send_http(&request, &network).await {
                Err(Verdict::Rejected(reason)) => {
                    assert!(reason.starts_with("destination denied"), "{reason}")
                }
                other => panic!("{url}: {other:?}"),
            }
        }
        let creds = HttpRequest::post("https://user:pw@example.com/");
        assert!(matches!(
            send_http(&creds, &network).await,
            Err(Verdict::Rejected(_))
        ));
    }
}
