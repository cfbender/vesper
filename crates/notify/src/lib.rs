//! Notification channel catalog: which channel kinds exist, which settings
//! each one takes and how a Vesper message becomes outbound requests.
//!
//! This crate is pure. Providers turn resolved [`Settings`] and a [`Message`]
//! into [`Request`]s and judge [`Response`]s; `vesper-engine` performs the
//! network IO under Vesper's destination policy, and `vesper-core` validates
//! channel configuration against each provider's [`Field`] list.

mod providers;
mod request;
mod settings;

pub use request::{
    body_code, by_status, HttpRequest, MailRequest, MailSecurity, Method, Request, RequestBody,
    Response, TokenStep, Verdict,
};
pub use settings::{Setting, Settings};

use chrono::{DateTime, Utc};

/// Default bound on composed message text, in characters.
pub const DEFAULT_MAX_CHARS: usize = 3800;

pub type Build = fn(&Settings, &Message) -> Result<Vec<Request>, String>;

/// One channel kind, such as `discord` or `ntfy`.
#[derive(Debug)]
pub struct Provider {
    /// The `kind:` value in configuration.
    pub kind: &'static str,
    /// Human-readable product name.
    pub name: &'static str,
    /// Upstream API documentation.
    pub docs: &'static str,
    pub fields: &'static [Field],
    /// Settings that identify the destination. Delivery state is kept per
    /// destination, so changing one of these makes open incidents pending
    /// for the new destination; changing any other setting (such as a
    /// rotated token) does not. Empty means every setting.
    pub identity: &'static [&'static str],
    /// Sends one incident per message, for incident-management APIs that
    /// deduplicate and resolve by key. [`Message::incident`] is then set.
    pub per_incident: bool,
    /// Bound on composed message text, in characters.
    pub max_chars: usize,
    /// Builds the requests for one message. They are sent in order and every
    /// one must be accepted. An error is a configuration problem.
    pub build: Build,
}

impl Provider {
    pub const fn new(
        kind: &'static str,
        name: &'static str,
        fields: &'static [Field],
        build: Build,
    ) -> Self {
        Self {
            kind,
            name,
            docs: "",
            fields,
            identity: &[],
            per_incident: false,
            max_chars: DEFAULT_MAX_CHARS,
            build,
        }
    }
    pub const fn docs(self, docs: &'static str) -> Self {
        Self { docs, ..self }
    }
    pub const fn identity(self, identity: &'static [&'static str]) -> Self {
        Self { identity, ..self }
    }
    pub const fn per_incident(self) -> Self {
        Self {
            per_incident: true,
            ..self
        }
    }
    pub const fn max_chars(self, max_chars: usize) -> Self {
        Self { max_chars, ..self }
    }

    pub fn field(&self, key: &str) -> Option<&'static Field> {
        self.fields.iter().find(|f| f.key == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    /// An absolute `http` or `https` URL.
    Url,
    Bool,
    Int,
    /// A YAML sequence of strings, or one string separated by commas or newlines.
    List,
    /// One of a fixed set of values.
    Choice(&'static [&'static str]),
}

/// One setting of a channel kind.
#[derive(Debug)]
pub struct Field {
    pub key: &'static str,
    pub kind: FieldKind,
    pub help: &'static str,
    pub required: bool,
    /// Credentials: only secret references (`{file: …}` or `{env: …}`) are
    /// accepted. Other settings take a literal or a secret reference.
    pub secret: bool,
    /// Value used when the setting is absent.
    pub default: Option<&'static str>,
    /// A plausible value for documentation and tests.
    pub example: Option<&'static str>,
}

impl Field {
    const fn new(key: &'static str, kind: FieldKind, help: &'static str) -> Self {
        Self {
            key,
            kind,
            help,
            required: true,
            secret: false,
            default: None,
            example: None,
        }
    }
    /// A required credential.
    pub const fn secret(key: &'static str, help: &'static str) -> Self {
        Self {
            secret: true,
            ..Self::new(key, FieldKind::Text, help)
        }
    }
    /// A required credential that is a URL, such as a webhook with a token in it.
    pub const fn secret_url(key: &'static str, help: &'static str) -> Self {
        Self {
            secret: true,
            ..Self::new(key, FieldKind::Url, help)
        }
    }
    pub const fn text(key: &'static str, help: &'static str) -> Self {
        Self::new(key, FieldKind::Text, help)
    }
    pub const fn url(key: &'static str, help: &'static str) -> Self {
        Self::new(key, FieldKind::Url, help)
    }
    /// Optional; false unless a default says otherwise.
    pub const fn bool(key: &'static str, help: &'static str) -> Self {
        Self {
            required: false,
            ..Self::new(key, FieldKind::Bool, help)
        }
    }
    pub const fn int(key: &'static str, help: &'static str) -> Self {
        Self::new(key, FieldKind::Int, help)
    }
    pub const fn list(key: &'static str, help: &'static str) -> Self {
        Self::new(key, FieldKind::List, help)
    }
    pub const fn choice(
        key: &'static str,
        values: &'static [&'static str],
        help: &'static str,
    ) -> Self {
        Self::new(key, FieldKind::Choice(values), help)
    }
    pub const fn optional(self) -> Self {
        Self {
            required: false,
            ..self
        }
    }
    pub const fn example(self, value: &'static str) -> Self {
        Self {
            example: Some(value),
            ..self
        }
    }
    /// Optional with this value when absent.
    pub const fn default(self, value: &'static str) -> Self {
        Self {
            required: false,
            default: Some(value),
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// At least one outage or reminder for a service that is still open.
    Down,
    /// Only recoveries.
    Up,
    /// A user-requested test message.
    Test,
}

/// The incident a per-incident message is about.
#[derive(Debug, Clone, PartialEq)]
pub struct IncidentRef {
    pub id: i64,
    pub service_id: String,
    pub service_name: String,
}

/// One composed notification. The text contains names, times, durations and
/// a dashboard link only; never probe evidence or credentials.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// Such as `Vesper: 1 new problem(s), 2 resolved`.
    pub title: String,
    /// One entry per line.
    pub lines: Vec<String>,
    /// Dashboard link for the incident or the incident list.
    pub link: String,
    pub status: Status,
    /// Set for providers with [`Provider::per_incident`], and never for tests.
    pub incident: Option<IncidentRef>,
    pub at: DateTime<Utc>,
}

impl Message {
    /// The entries without title or link.
    pub fn summary(&self) -> String {
        self.lines.join("\n")
    }
    /// The entries followed by the link, for providers with a separate title.
    pub fn body(&self) -> String {
        format!("{}\n{}", self.summary(), self.link)
    }
    /// Title, entries and link as one plain text.
    pub fn text(&self) -> String {
        format!("{}\n{}", self.title, self.body())
    }
    /// Stable key for deduplicating and resolving: per incident when the
    /// message is about one, otherwise a fixed key per status.
    pub fn dedup_key(&self) -> String {
        match (&self.incident, self.status) {
            (Some(i), _) => format!("vesper/incident/{}", i.id),
            (None, Status::Test) => "vesper/test".into(),
            (None, _) => "vesper/summary".into(),
        }
    }
    /// `DOWN`, `UP` or `TEST`.
    pub fn status_word(&self) -> &'static str {
        match self.status {
            Status::Down => "DOWN",
            Status::Up => "UP",
            Status::Test => "TEST",
        }
    }
}

/// Every channel kind, sorted by `kind`.
pub fn providers() -> &'static [&'static Provider] {
    providers::ALL
}

pub fn provider(kind: &str) -> Option<&'static Provider> {
    providers::ALL.iter().copied().find(|p| p.kind == kind)
}
