//! Outbound requests built by providers and how their responses are judged.

use base64::Engine as _;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestBody {
    Empty,
    Json(Value),
    /// `application/x-www-form-urlencoded`.
    Form(Vec<(String, String)>),
    /// Raw text with its content type.
    Text {
        content_type: String,
        text: String,
    },
}

/// Fetches a bearer token before the request it belongs to, for APIs that
/// exchange client credentials for a short-lived token.
#[derive(Debug, Clone)]
pub struct TokenStep {
    pub request: HttpRequest,
    /// JSON pointer to the token in the response, such as `/access_token`.
    pub pointer: &'static str,
    /// Header that carries the token on the main request.
    pub header: &'static str,
    /// Prefix before the token, such as `Bearer `.
    pub prefix: &'static str,
}

/// One HTTP request. The URL must be absolute `http` or `https`.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: RequestBody,
    /// Judges the response; [`by_status`] unless the API reports errors in
    /// the body of a successful response.
    pub accept: fn(&Response) -> Verdict,
    pub token: Option<Box<TokenStep>>,
}

impl HttpRequest {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: vec![],
            body: RequestBody::Empty,
            accept: by_status,
            token: None,
        }
    }
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(Method::Get, url)
    }
    pub fn post(url: impl Into<String>) -> Self {
        Self::new(Method::Post, url)
    }
    pub fn put(url: impl Into<String>) -> Self {
        Self::new(Method::Put, url)
    }
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }
    pub fn basic(self, user: &str, password: &str) -> Self {
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        self.header("Authorization", format!("Basic {encoded}"))
    }
    pub fn json(mut self, body: Value) -> Self {
        self.body = RequestBody::Json(body);
        self
    }
    pub fn form<K: Into<String>, V: Into<String>>(
        mut self,
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.body = RequestBody::Form(
            pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
        self
    }
    pub fn text(mut self, content_type: &str, text: impl Into<String>) -> Self {
        self.body = RequestBody::Text {
            content_type: content_type.into(),
            text: text.into(),
        };
        self
    }
    /// Appends query parameters, percent-encoded.
    pub fn query<K: AsRef<str>, V: AsRef<str>>(
        mut self,
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self, String> {
        let mut url = url::Url::parse(&self.url).map_err(|_| "invalid URL".to_string())?;
        url.query_pairs_mut().extend_pairs(pairs);
        self.url = url.into();
        Ok(self)
    }
    pub fn accept(mut self, accept: fn(&Response) -> Verdict) -> Self {
        self.accept = accept;
        self
    }
    /// Fetches a token with `request` first and sends it as
    /// `Authorization: Bearer <token>`, taken from `pointer` in its JSON.
    pub fn bearer_from(mut self, request: HttpRequest, pointer: &'static str) -> Self {
        self.token = Some(Box::new(TokenStep {
            request,
            pointer,
            header: "Authorization",
            prefix: "Bearer ",
        }));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailSecurity {
    /// Implicit TLS, usually port 465.
    Tls,
    /// STARTTLS required, usually port 587.
    StartTls,
    /// Plain SMTP, for relays on a trusted network.
    None,
}

/// One email sent over SMTP.
#[derive(Debug, Clone, PartialEq)]
pub struct MailRequest {
    pub host: String,
    pub port: u16,
    pub security: MailSecurity,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    /// Plain text.
    pub body: String,
}

#[derive(Debug, Clone)]
pub enum Request {
    Http(HttpRequest),
    Mail(MailRequest),
}

impl From<HttpRequest> for Request {
    fn from(r: HttpRequest) -> Self {
        Request::Http(r)
    }
}

impl From<MailRequest> for Request {
    fn from(r: MailRequest) -> Self {
        Request::Mail(r)
    }
}

/// HTTP response: status, `Retry-After` in seconds and bounded body bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Response {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }
    pub fn success(&self) -> bool {
        (200..300).contains(&self.status)
    }
    /// A short description from the body: a common JSON error field, or the
    /// start of the text. The engine scrubs credentials from it.
    pub fn description(&self) -> String {
        let json = self.json();
        for key in [
            "description",
            "message",
            "error_description",
            "error",
            "errmsg",
            "msg",
            "detail",
            "errors",
        ] {
            match json.get(key) {
                Some(Value::String(s)) if !s.is_empty() => return s.clone(),
                Some(Value::Object(o)) => {
                    if let Some(Value::String(s)) = o.get("message") {
                        return s.clone();
                    }
                }
                Some(v @ Value::Array(_)) => return v.to_string(),
                _ => {}
            }
        }
        let text = String::from_utf8_lossy(&self.body);
        let text = text.trim();
        if text.is_empty() || text.starts_with('<') {
            String::new()
        } else {
            text.chars().take(120).collect()
        }
    }
    /// `HTTP 403: description`, or `HTTP 403` without one. The engine
    /// prefixes verdict texts with the provider name.
    pub fn summary(&self) -> String {
        match self.description() {
            d if d.is_empty() => format!("HTTP {}", self.status),
            d => format!("HTTP {}: {d}", self.status),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Accepted,
    /// The service asked to wait this many seconds.
    RetryAfter(u64),
    /// Authentication, permission or configuration rejection; not retried
    /// until the configuration changes or a test message succeeds.
    Rejected(String),
    /// Server or network failure; retried with backoff.
    Transient(String),
}

/// Judges by status code: 2xx accepted, 429 rate limited, other 4xx except
/// 408 rejected, anything else transient.
pub fn by_status(r: &Response) -> Verdict {
    match r.status {
        200..=299 => Verdict::Accepted,
        429 => Verdict::RetryAfter(r.retry_after.unwrap_or(30).max(1)),
        408 => Verdict::Transient(r.summary()),
        400..=499 => Verdict::Rejected(r.summary()),
        _ => Verdict::Transient(r.summary()),
    }
}

/// For APIs that answer 200 with an error code in the body: accepted when
/// the status is 2xx and the JSON at `pointer` equals `expected`, comparing
/// numbers and strings by their text (so `0` matches `"0"`).
pub fn body_code(r: &Response, pointer: &str, expected: &str) -> Verdict {
    match by_status(r) {
        Verdict::Accepted => {
            let text = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_string);
            match r.json().pointer(pointer) {
                Some(v) if text(v) == expected => Verdict::Accepted,
                _ => Verdict::Rejected(r.summary()),
            }
        }
        other => other,
    }
}
