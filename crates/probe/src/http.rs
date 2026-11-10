//! HTTP(S) probe: one GET per hop over a socket connected to a validated
//! address, HTTP/1.1 only, no connection reuse and no proxy.

use crate::policy::AddressPolicy;
use crate::{connect, resolve_secret, Failure, Target};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper::header::{self, HeaderName, HeaderValue};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;
use url::{Host, Position, Url};
use vesper_core::{Check, NetworkPolicy, SecretRef};

const MAX_REDIRECTS: usize = 5;
/// Bound on the buffered response head (status line and headers).
const MAX_HEAD_BYTES: usize = 64 * 1024;
const USER_AGENT: &str = concat!("vesper-probe/", env!("CARGO_PKG_VERSION"));
/// Headers the probe owns; configuring them could desynchronize framing or routing.
const RESERVED_HEADERS: [HeaderName; 6] = [
    header::HOST,
    header::CONNECTION,
    header::CONTENT_LENGTH,
    header::TRANSFER_ENCODING,
    header::UPGRADE,
    header::TE,
];

pub(crate) async fn check(
    check: &Check,
    network: &NetworkPolicy,
    policy: &AddressPolicy,
    secrets_root: &Path,
) -> Result<String, Failure> {
    let raw = check
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| Failure::new("HTTP check has no url"))?;
    let original = Url::parse(raw).map_err(|_| Failure::new("HTTP check url is invalid"))?;
    validate_url(&original)?;
    let secret_headers = resolve_headers(check, secrets_root)?;

    let mut tls = None;
    let mut url = original.clone();
    let mut send_secrets = true;
    let mut redirects = 0;
    loop {
        // Once a redirect leaves the configured origin, secrets stay behind
        // for the rest of the chain.
        send_secrets &= url.origin() == original.origin();
        let headers = if send_secrets {
            &secret_headers[..]
        } else {
            &[]
        };
        let exchange = fetch(&url, network, policy, secrets_root, &mut tls, headers).await?;
        let status = exchange.response.status();

        if check.follow_redirects && is_followed_redirect(status) {
            if redirects == MAX_REDIRECTS {
                return Err(Failure::new(format!(
                    "stopped after {MAX_REDIRECTS} redirects"
                )));
            }
            let next = exchange
                .response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|location| url.join(location).ok())
                .ok_or_else(|| {
                    Failure::new(format!(
                        "HTTP {} redirect from {} has a missing or invalid Location",
                        status.as_u16(),
                        exchange.peer
                    ))
                })?;
            validate_url(&next)
                .map_err(|Failure(reason)| Failure::new(format!("refused redirect: {reason}")))?;
            if url.scheme() == "https" && next.scheme() == "http" {
                return Err(Failure::new("refused redirect from HTTPS to HTTP"));
            }
            url = next;
            redirects += 1;
            continue;
        }

        let peer = exchange.peer;
        let via = match redirects {
            0 => String::new(),
            1 => " after 1 redirect".into(),
            n => format!(" after {n} redirects"),
        };
        if status.as_u16() != check.expected_status {
            return Err(Failure::new(format!(
                "expected HTTP {}, got {} from {peer}{via}",
                check.expected_status,
                status.as_u16()
            )));
        }
        let Some(needle) = check.body_contains.as_deref() else {
            return Ok(format!("HTTP {} from {peer}{via}", status.as_u16()));
        };
        let body = read_bounded(exchange.response, network.max_response_bytes, peer).await?;
        return if contains(&body, needle.as_bytes()) {
            Ok(format!(
                "HTTP {} from {peer}{via}; body matched",
                status.as_u16()
            ))
        } else {
            Err(Failure::new(format!(
                "HTTP {} from {peer}{via}, but the body did not contain the expected text",
                status.as_u16()
            )))
        };
    }
}

fn validate_url(url: &Url) -> Result<(), Failure> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Failure::new("only http and https URLs are supported"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Failure::new(
            "URLs must not contain credentials; use secret header references",
        ));
    }
    if url.host().is_none() {
        return Err(Failure::new("URL has no host"));
    }
    Ok(())
}

fn is_followed_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn resolve_headers(
    check: &Check,
    secrets_root: &Path,
) -> Result<Vec<(HeaderName, HeaderValue)>, Failure> {
    let mut headers = Vec::with_capacity(check.headers.len());
    for (name, secret) in &check.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| Failure::new(format!("header name `{name}` is invalid")))?;
        if RESERVED_HEADERS.contains(&name) {
            return Err(Failure::new(format!(
                "header `{name}` cannot be configured"
            )));
        }
        let value = resolve_secret(secret, secrets_root)
            .map_err(|reason| Failure::new(format!("header `{name}`: {reason}")))?;
        let mut value = HeaderValue::from_str(&value).map_err(|_| {
            Failure::new(format!(
                "secret for header `{name}` is not a valid header value"
            ))
        })?;
        value.set_sensitive(true);
        headers.push((name, value));
    }
    Ok(headers)
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// Aborts the connection driver when the exchange (or the whole probe) is dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Exchange {
    response: Response<Incoming>,
    peer: SocketAddr,
    _connection: AbortOnDrop,
}

async fn fetch(
    url: &Url,
    network: &NetworkPolicy,
    policy: &AddressPolicy,
    secrets_root: &Path,
    tls: &mut Option<Arc<ClientConfig>>,
    secret_headers: &[(HeaderName, HeaderValue)],
) -> Result<Exchange, Failure> {
    let host = url.host().ok_or_else(|| Failure::new("URL has no host"))?;
    let target = match host {
        Host::Domain(name) => Target::Name(name.to_ascii_lowercase()),
        Host::Ipv4(ip) => Target::Ip(IpAddr::V4(ip)),
        Host::Ipv6(ip) => Target::Ip(IpAddr::V6(ip)),
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Failure::new("URL has no port"))?;
    let (stream, peer) = connect(&target, port, policy).await?;

    let io: Box<dyn Io> = if url.scheme() == "https" {
        let config = match tls {
            Some(config) => config.clone(),
            None => tls.insert(tls_config(network, secrets_root)?).clone(),
        };
        let server_name = match host {
            Host::Domain(name) => ServerName::try_from(name.to_string())
                .map_err(|_| Failure::new(format!("{target} is not a valid TLS server name")))?,
            Host::Ipv4(ip) => ServerName::IpAddress(IpAddr::V4(ip).into()),
            Host::Ipv6(ip) => ServerName::IpAddress(IpAddr::V6(ip).into()),
        };
        let stream = TlsConnector::from(config)
            .connect(server_name, stream)
            .await
            .map_err(|error| tls_failure(&error, peer))?;
        Box::new(stream)
    } else {
        Box::new(stream)
    };

    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_buf_size(MAX_HEAD_BYTES)
        .handshake::<_, Empty<Bytes>>(TokioIo::new(io))
        .await
        .map_err(|error| http_failure(&error, peer))?;
    let connection = AbortOnDrop(tokio::spawn(async move {
        let _ = connection.await;
    }));

    let authority = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    };
    let mut request = Request::get(&url[Position::BeforePath..Position::AfterQuery])
        .header(header::HOST, authority)
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::ACCEPT, "*/*")
        .header(header::CONNECTION, "close")
        .body(Empty::new())
        .map_err(|_| Failure::new("URL cannot be sent as an HTTP request"))?;
    for (name, value) in secret_headers {
        request.headers_mut().insert(name.clone(), value.clone());
    }
    let response = sender
        .send_request(request)
        .await
        .map_err(|error| http_failure(&error, peer))?;
    Ok(Exchange {
        response,
        peer,
        _connection: connection,
    })
}

async fn read_bounded(
    response: Response<Incoming>,
    limit: usize,
    peer: SocketAddr,
) -> Result<Vec<u8>, Failure> {
    let too_large = || Failure::new(format!("response body from {peer} exceeded {limit} bytes"));
    let declared = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if declared.is_some_and(|length| length > limit as u64) {
        return Err(too_large());
    }
    let mut body = response.into_body();
    let mut buffer = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| http_failure(&error, peer))?;
        if let Ok(data) = frame.into_data() {
            if buffer.len() + data.len() > limit {
                return Err(too_large());
            }
            buffer.extend_from_slice(&data);
        }
    }
    Ok(buffer)
}

/// Public web roots plus the optional private CA, which is a file under the
/// secrets root with the same confinement as a `{file: name}` secret.
fn tls_config(network: &NetworkPolicy, secrets_root: &Path) -> Result<Arc<ClientConfig>, Failure> {
    let mut roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    if let Some(file) = &network.private_ca_file {
        let pem = resolve_secret(&SecretRef::File(file.clone()), secrets_root)
            .map_err(|reason| Failure::new(format!("private CA: {reason}")))?;
        let mut added = 0;
        for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
            let cert = cert.map_err(|_| Failure::new("private CA file is not valid PEM"))?;
            roots
                .add(cert)
                .map_err(|_| Failure::new("private CA file contains an invalid certificate"))?;
            added += 1;
        }
        if added == 0 {
            return Err(Failure::new("private CA file contains no certificates"));
        }
    }
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|_| Failure::new("TLS configuration failed"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

fn tls_failure(error: &std::io::Error, peer: SocketAddr) -> Failure {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(tls) => Failure::new(format!("TLS handshake with {peer} failed: {tls}")),
        None => Failure::new(format!(
            "TLS handshake with {peer} failed: {}",
            error.kind()
        )),
    }
}

fn http_failure(error: &hyper::Error, peer: SocketAddr) -> Failure {
    let reason = if error.is_parse() || error.is_parse_status() {
        // Includes "message head is too large" past MAX_HEAD_BYTES.
        format!("invalid HTTP response ({error})")
    } else if error.is_incomplete_message() {
        "connection closed before the response completed".into()
    } else {
        // hyper's own descriptions carry no response data.
        error.to_string()
    };
    Failure::new(format!("HTTP exchange with {peer} failed: {reason}"))
}
