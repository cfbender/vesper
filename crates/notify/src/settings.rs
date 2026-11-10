//! Resolved channel settings as providers read them.

use crate::{Field, Provider};
use std::collections::BTreeMap;

/// A resolved setting value. Booleans and integers arrive as their text.
#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Text(String),
    List(Vec<String>),
}

/// Resolved settings of one channel. Secret references have been replaced by
/// their values, so a `Settings` must never be logged or shown.
pub struct Settings {
    provider: &'static Provider,
    values: BTreeMap<String, Setting>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("kind", &self.provider.kind)
            .field("keys", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Settings {
    pub fn new(provider: &'static Provider, values: BTreeMap<String, Setting>) -> Self {
        Self { provider, values }
    }

    pub fn provider(&self) -> &'static Provider {
        self.provider
    }

    pub fn values(&self) -> &BTreeMap<String, Setting> {
        &self.values
    }

    fn field(&self, key: &str) -> Option<&'static Field> {
        let field = self.provider.field(key);
        debug_assert!(
            field.is_some(),
            "{} reads undeclared setting {key}",
            self.provider.kind
        );
        field
    }

    /// The trimmed value, or the field default; `None` when empty or absent.
    pub fn get(&self, key: &str) -> Option<&str> {
        let field = self.field(key)?;
        let value = match self.values.get(key) {
            Some(Setting::Text(t)) => Some(t.trim()),
            Some(Setting::List(_)) => None,
            None => None,
        };
        value.filter(|v| !v.is_empty()).or(field.default)
    }

    /// A value the request cannot be built without.
    pub fn str(&self, key: &str) -> Result<&str, String> {
        self.get(key)
            .ok_or_else(|| format!("setting {key} is required"))
    }

    /// A URL without trailing slashes, for joining paths onto.
    pub fn base_url(&self, key: &str) -> Result<String, String> {
        Ok(self.str(key)?.trim_end_matches('/').to_string())
    }

    pub fn bool(&self, key: &str) -> bool {
        self.get(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    pub fn int(&self, key: &str) -> Result<Option<i64>, String> {
        self.get(key)
            .map(|v| {
                v.parse()
                    .map_err(|_| format!("setting {key} must be an integer"))
            })
            .transpose()
    }

    /// Items of a list setting; a text value is split on commas, semicolons
    /// and newlines. Empty items are dropped.
    pub fn list(&self, key: &str) -> Vec<String> {
        if self.field(key).is_none() {
            return vec![];
        }
        let items: Vec<String> = match self.values.get(key) {
            Some(Setting::List(items)) => items.clone(),
            Some(Setting::Text(t)) => t.split([',', ';', '\n']).map(str::to_string).collect(),
            None => vec![],
        };
        items
            .into_iter()
            .map(|i| i.trim().to_string())
            .filter(|i| !i.is_empty())
            .collect()
    }

    /// Like [`Settings::list`], but at least one item is required.
    pub fn required_list(&self, key: &str) -> Result<Vec<String>, String> {
        let items = self.list(key);
        if items.is_empty() {
            return Err(format!("setting {key} needs at least one entry"));
        }
        Ok(items)
    }
}
