use crate::App;
use anyhow::{bail, Context, Result};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use ipnet::IpNet;
use rand::RngCore;
use std::{
    env,
    net::{IpAddr, SocketAddr},
};
use subtle::ConstantTimeEq;
use url::Url;

#[derive(Clone)]
pub struct Security {
    pub origin: String,
    pub trusted_proxies: Vec<IpNet>,
    pub csrf: String,
    pub dev_mode: bool,
}

impl Security {
    pub fn from_env() -> Result<Self> {
        let dev_mode = env::var("VESPER_DEV_MODE").as_deref() == Ok("true");
        let public_url = env::var("VESPER_PUBLIC_URL").unwrap_or_else(|_| {
            if dev_mode {
                "http://localhost:8080".into()
            } else {
                String::new()
            }
        });
        let url =
            Url::parse(&public_url).context("Set VESPER_PUBLIC_URL to the public HTTPS origin")?;
        if (!dev_mode && url.scheme() != "https")
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            bail!("VESPER_PUBLIC_URL must be an HTTPS origin without credentials, path, query or fragment");
        }
        let trusted_proxies = env::var("VESPER_TRUSTED_PROXIES")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                s.trim()
                    .parse::<IpNet>()
                    .context("VESPER_TRUSTED_PROXIES contains an invalid CIDR")
            })
            .collect::<Result<Vec<_>>>()?;
        if !dev_mode && trusted_proxies.is_empty() {
            bail!("Set VESPER_TRUSTED_PROXIES to the authentication proxy's private address CIDRs; no dashboard bypass is permitted");
        }
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let csrf = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            origin: url.origin().ascii_serialization(),
            trusted_proxies,
            csrf,
            dev_mode,
        })
    }

    fn trusts(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
            _ => ip,
        };
        self.dev_mode || self.trusted_proxies.iter().any(|cidr| cidr.contains(&ip))
    }
}

pub fn equal_secret(left: &str, right: &str) -> bool {
    !left.is_empty() && bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

pub async fn protect(State(app): State<App>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let heartbeat = request.method() == Method::POST
        && path.strip_prefix("/api/heartbeat/").is_some_and(|suffix| {
            suffix.split('/').count() == 3 && suffix.split('/').all(valid_id)
        });
    if path != "/healthz" && !heartbeat {
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip());
        if !peer.is_some_and(|ip| app.security.trusts(ip)) {
            return denied("Access requires the configured authentication proxy");
        }
        if request
            .headers()
            .get("sec-fetch-site")
            .and_then(|v| v.to_str().ok())
            == Some("cross-site")
        {
            return denied("Cross-site requests are not allowed");
        }
        let safe_method = matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        );
        if !safe_method {
            let origin = request
                .headers()
                .get(header::ORIGIN)
                .and_then(|v| v.to_str().ok());
            let csrf = request
                .headers()
                .get("x-vesper-csrf")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            if origin != Some(app.security.origin.as_str())
                || !equal_secret(csrf, &app.security.csrf)
            {
                return denied("Mutation requires the configured Origin and X-Vesper-CSRF token");
            }
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if !app.security.dev_mode {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    response
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn denied(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"error": message})),
    )
        .into_response()
}
