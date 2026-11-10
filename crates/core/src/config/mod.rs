//! Directory-backed YAML configuration: loading, validation, provenance and
//! safe, source-aware saving. See `docs/configuration.md` for the layout.

use crate::model::{Config, Machine, Service};
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// A validated configuration together with where each part came from.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    pub config: Config,
    /// Opaque content revision (`sha256:<hex>`); changes whenever any
    /// configuration file or the directory layout changes.
    pub revision: String,
    /// Logical path (`machines/jelly/services/media`) to the root-relative
    /// YAML file that defines it.
    pub sources: BTreeMap<String, String>,
    /// Whether Vesper may write to the configuration root.
    pub writable: bool,
}

/// Errors from save operations that callers map to distinct responses.
/// Retrieve with `err.downcast_ref::<SaveError>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The configuration changed since `expected_revision` was read.
    Conflict { current_revision: String },
    /// The configuration root is not writable; export instead.
    ReadOnly,
    /// The edit cannot be applied to the existing file layout without
    /// losing data or comments; export the YAML and edit the files by hand.
    ExportOnly { reason: String },
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Conflict { .. } => write!(
                f,
                "configuration changed since it was loaded; reload and reapply the edit"
            ),
            SaveError::ReadOnly => write!(
                f,
                "configuration is read-only; export the YAML and edit the files instead"
            ),
            SaveError::ExportOnly { reason } => write!(
                f,
                "this edit cannot be saved safely ({reason}); export the YAML and edit the files instead"
            ),
        }
    }
}

impl std::error::Error for SaveError {}

/// Loads and validates the configuration rooted at the directory `root`.
pub fn load(_root: &Path) -> Result<LoadedConfig> {
    bail!("configuration loading is not implemented yet")
}

/// Validates a configuration assembled from any source.
pub fn validate(_config: &Config) -> Result<()> {
    Ok(())
}

/// Saves one service into the file that owns it.
pub fn save_service(
    _root: &Path,
    _expected_revision: &str,
    _machine_id: &str,
    _service_id: &str,
    _service: &Service,
) -> Result<LoadedConfig> {
    bail!("configuration saving is not implemented yet")
}

/// Saves machine fields, or creates a new machine.
pub fn save_machine(
    _root: &Path,
    _expected_revision: &str,
    _machine_id: &str,
    _machine: &Machine,
) -> Result<LoadedConfig> {
    bail!("configuration saving is not implemented yet")
}

/// Renders a service as standalone YAML (the content of `services/<id>.yaml`).
pub fn export_service(_service: &Service) -> Result<String> {
    bail!("export is not implemented yet")
}
