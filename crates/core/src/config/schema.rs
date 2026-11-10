//! Where each part of the configuration sits in the model, which layouts it
//! permits, and how it is written back (key order and omitted defaults).

use super::yaml::Out;
use crate::model::{Check, Machine, Service};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pos {
    Root,
    Machines,
    Machine,
    Services,
    Service,
    Checks,
    Check,
    Maintenance,
    Window,
    Leaf,
}

impl Pos {
    pub fn child(self, key: &str) -> Pos {
        match (self, key) {
            (Pos::Root, "machines") => Pos::Machines,
            (Pos::Machines, _) => Pos::Machine,
            (Pos::Machine, "services") => Pos::Services,
            (Pos::Services, _) => Pos::Service,
            (Pos::Service, "checks") => Pos::Checks,
            (Pos::Service, "maintenance") => Pos::Maintenance,
            (Pos::Checks, _) => Pos::Check,
            (Pos::Maintenance, _) => Pos::Window,
            _ => Pos::Leaf,
        }
    }

    /// ID-keyed mappings whose keys are machine, service or check IDs.
    pub fn is_collection(self) -> bool {
        matches!(self, Pos::Machines | Pos::Services | Pos::Checks)
    }

    /// Positions that may be represented by a directory.
    pub fn allows_dir(self) -> bool {
        matches!(
            self,
            Pos::Root | Pos::Machines | Pos::Machine | Pos::Services | Pos::Service | Pos::Checks
        )
    }

    /// Field names that may be split into `<field>.yaml` or `<field>/` inside
    /// a struct directory. Other fields belong in `_index.yaml`.
    pub fn split_fields(self) -> &'static [&'static str] {
        match self {
            Pos::Root => &[
                "defaults",
                "network",
                "telegram",
                "notifications",
                "machines",
            ],
            Pos::Machine => &["services"],
            Pos::Service => &["checks"],
            _ => &[],
        }
    }

    /// An empty file at this position means an empty mapping.
    pub fn is_mapping(self) -> bool {
        self.allows_dir() || matches!(self, Pos::Check | Pos::Window)
    }

    fn defaults(self) -> Option<Value> {
        let v = match self {
            Pos::Machine => serde_json::to_value(Machine::default()),
            Pos::Service => serde_json::to_value(Service::default()),
            Pos::Check => serde_json::to_value(Check::default()),
            Pos::Window => Ok(serde_json::json!({ "repeat_weekly": false })),
            _ => return None,
        };
        Some(v.expect("default serialization"))
    }

    /// Fields always written even when they equal their default.
    fn always(self) -> &'static [&'static str] {
        match self {
            Pos::Check => &["kind"],
            _ => &[],
        }
    }

    /// Model field order, used when writing new keys.
    pub fn order(self) -> &'static [&'static str] {
        match self {
            Pos::Root => &[
                "defaults",
                "network",
                "telegram",
                "notifications",
                "machines",
            ],
            Pos::Machine => &["name", "site", "description", "host_service", "services"],
            Pos::Service => &[
                "name",
                "icon",
                "description",
                "paused",
                "notifications",
                "depends_on",
                "maintenance",
                "checks",
            ],
            Pos::Check => &[
                "kind",
                "name",
                "enabled",
                "url",
                "host",
                "port",
                "expected_status",
                "body_contains",
                "headers",
                "follow_redirects",
                "token",
                "expected_interval_seconds",
                "grace_seconds",
                "interval_seconds",
                "timeout_seconds",
                "failure_threshold",
                "recovery_threshold",
                "stale_after_seconds",
            ],
            Pos::Window => &["start", "end", "repeat_weekly"],
            _ => &[],
        }
    }

    /// Whether a field absent from a file may stay absent for this value.
    pub fn omit(self, key: &str, value: &Value) -> bool {
        value.is_null()
            || (!self.always().contains(&key)
                && self.defaults().is_some_and(|d| d.get(key) == Some(value)))
    }

    /// Orders keys of an object at this position: model order first, then
    /// the remaining keys sorted.
    pub fn ordered_keys<'a>(self, keys: impl Iterator<Item = &'a String>) -> Vec<String> {
        let mut keys: Vec<String> = keys.cloned().collect();
        let order = self.order();
        keys.sort_by_key(|k| {
            (
                order.iter().position(|o| o == k).unwrap_or(usize::MAX),
                k.clone(),
            )
        });
        keys
    }
}

/// Converts a typed value into its minimal written form for `pos`.
pub fn shape(pos: Pos, v: &Value) -> Out {
    match v {
        Value::Object(o) => {
            let keys = pos.ordered_keys(o.keys().filter(|k| !pos.omit(k, &o[*k])));
            Out::Map(
                keys.into_iter()
                    .map(|k| {
                        let out = shape(pos.child(&k), &o[&k]);
                        (k, out)
                    })
                    .collect(),
            )
        }
        Value::Array(items) => Out::Seq(items.iter().map(|i| shape(pos.child(""), i)).collect()),
        other => Out::Scalar(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_lists_cover_every_model_field() {
        for pos in [Pos::Machine, Pos::Service, Pos::Check] {
            let d = pos.defaults().unwrap();
            for k in d.as_object().unwrap().keys() {
                assert!(pos.order().contains(&k.as_str()), "{pos:?} misses {k}");
            }
            assert_eq!(pos.order().len(), d.as_object().unwrap().len());
        }
        let root = serde_json::to_value(crate::Config::default()).unwrap();
        assert_eq!(root.as_object().unwrap().len(), Pos::Root.order().len());
        let w = serde_json::to_value(crate::MaintenanceWindow {
            start: String::new(),
            end: String::new(),
            repeat_weekly: false,
        })
        .unwrap();
        assert_eq!(w.as_object().unwrap().len(), Pos::Window.order().len());
    }
}
