//! Directory-backed YAML configuration: loading, validation, provenance and
//! safe, source-aware saving. See `crates/core/CONFIGURATION.md` for the layout,
//! rules and limitations.

mod assemble;
mod fs;
mod save;
mod schema;
mod validate;
mod yaml;

use crate::model::{Config, Machine, Service};
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

pub use assemble::sanitize_serde;
pub use validate::validate;

/// Root file with settings taken from the Vesper container's labels. It
/// holds a root-shaped mapping without services, and every other file is
/// deep-merged over it: a value set in any other file wins.
pub const LABELS_FILE: &str = "_labels.yaml";

/// Maximum length of a machine, service or check ID, in bytes.
pub const MAX_ID_BYTES: usize = 80;

/// Whether `id` is a valid machine, service or check ID: 1 to
/// [`MAX_ID_BYTES`] ASCII letters, digits, `-` and `_`, and not `_index`.
pub fn is_valid_id(id: &str) -> bool {
    assemble::is_valid_id(id)
}

/// A validated configuration together with where each part came from.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    pub config: Config,
    /// Opaque content revision (`sha256:<hex>`); changes whenever any
    /// configuration file or the directory layout changes.
    pub revision: String,
    /// Logical path (`machines/jelly/services/media`) to the root-relative
    /// YAML file that defines it. Machines, services, checks and the
    /// `defaults`, `network`, `telegram` and `notifications` sections are listed. A path
    /// ending in `/` is a directory without an `_index.yaml`.
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
///
/// Returns an error (and nothing partial) when any file is malformed, the
/// layout is ambiguous, or validation fails; callers keep their last valid
/// [`LoadedConfig`] in that case.
pub fn load(root: &Path) -> Result<LoadedConfig> {
    let root = fs::Root::open(root)?;
    let tree = {
        let _lock = root.lock(false)?;
        root.scan()?
    };
    loaded(&tree)
}

fn loaded(tree: &fs::Tree) -> Result<LoadedConfig> {
    let (assembled, config) = assemble::assemble(tree)?;
    validate(&config)?;
    Ok(LoadedConfig {
        config,
        revision: tree.revision(),
        sources: assembled.sources(),
        writable: tree.writable,
    })
}

/// Saves one service (new or existing) on an existing machine, editing only
/// the files that define it.
pub fn save_service(
    root: &Path,
    expected_revision: &str,
    machine_id: &str,
    service_id: &str,
    service: &Service,
) -> Result<LoadedConfig> {
    if !is_valid_id(machine_id) || !is_valid_id(service_id) {
        bail!("machine and service IDs use ASCII letters, digits, '-' and '_' (at most {MAX_ID_BYTES} bytes)");
    }
    save_with(root, expected_revision, |config| {
        let machine = config.machines.get_mut(machine_id).with_context(|| {
            format!("machine `{machine_id}` does not exist; save the machine first")
        })?;
        machine
            .services
            .insert(service_id.to_string(), service.clone());
        Ok(())
    })
}

/// Creates a machine (including any services it carries) or updates an
/// existing machine's own fields. For an existing machine `machine.services`
/// must be empty or identical to the current services; services are
/// changed with [`save_service`].
pub fn save_machine(
    root: &Path,
    expected_revision: &str,
    machine_id: &str,
    machine: &Machine,
) -> Result<LoadedConfig> {
    if !is_valid_id(machine_id) {
        bail!("machine IDs use ASCII letters, digits, '-' and '_' (at most {MAX_ID_BYTES} bytes)");
    }
    save_with(root, expected_revision, |config| {
        match config.machines.get_mut(machine_id) {
            Some(current) => {
                if !machine.services.is_empty() && machine.services != current.services {
                    bail!("saving a machine does not change its services; save each service separately");
                }
                let services = std::mem::take(&mut current.services);
                *current = Machine {
                    services,
                    ..machine.clone()
                };
            }
            None => {
                config
                    .machines
                    .insert(machine_id.to_string(), machine.clone());
            }
        }
        Ok(())
    })
}

fn save_with(
    root: &Path,
    expected_revision: &str,
    change: impl FnOnce(&mut Config) -> Result<()>,
) -> Result<LoadedConfig> {
    let root = fs::Root::open(root)?;
    let _lock = root.lock(true)?;
    let tree = root.scan()?;
    {
        if !tree.writable {
            return Err(SaveError::ReadOnly.into());
        }
        let revision = tree.revision();
        if revision != expected_revision {
            return Err(SaveError::Conflict {
                current_revision: revision,
            }
            .into());
        }
        let (assembled, current) = assemble::assemble(&tree)?;
        validate(&current).context("the configuration on disk is invalid; fix it before saving")?;
        let mut next = current.clone();
        change(&mut next)?;
        validate(&next)?;
        let old_v = serde_json::to_value(&current)?;
        let new_v = serde_json::to_value(&next)?;
        let ops = save::plan(&assembled, &old_v, &new_v)?;
        // Prove the edit before touching disk: the edited tree must assemble
        // into exactly the requested configuration.
        let mut edited = tree.clone();
        edited.apply(&ops);
        edited.check_bounds()?;
        match assemble::assemble(&edited) {
            Ok((_, check)) if check == next => {}
            _ => {
                return Err(SaveError::ExportOnly {
                    reason: "the edited files could not be verified to produce this configuration"
                        .into(),
                }
                .into())
            }
        }
        root.commit(&tree, &ops)?;
    }
    loaded(&root.scan()?)
}

/// A whole-file change planned for [`edit_files`]. Paths are root-relative
/// `.yaml` files such as `machines/docker1/services/web.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChange {
    Write { path: String, content: String },
    Delete { path: String },
}

/// Replaces or deletes whole files, for tools that own entire files (such as
/// `vesper discover`) rather than editing values inside them.
///
/// Under the same exclusive lock as the save functions, `plan` receives the
/// current validated configuration and the text of every file, and returns
/// the changes together with the configuration they must produce. Nothing is
/// written unless the edited tree assembles into exactly that configuration;
/// missing parent directories are created. External edits during the commit
/// return [`SaveError::Conflict`]. With `dry_run` the edit is verified the
/// same way but never written, and the root need not be writable.
///
/// The planner's third argument previews changes: it returns the validated
/// configuration they would produce, or why they cannot be applied.
pub fn edit_files(
    root: &Path,
    dry_run: bool,
    plan: impl FnOnce(
        &LoadedConfig,
        &BTreeMap<String, String>,
        &dyn Fn(&[FileChange]) -> Result<LoadedConfig>,
    ) -> Result<(Vec<FileChange>, Config)>,
) -> Result<LoadedConfig> {
    let root = fs::Root::open(root)?;
    let _lock = root.lock(true)?;
    let tree = root.scan()?;
    let current =
        loaded(&tree).context("the configuration on disk is invalid; fix it before editing")?;
    let preview = |changes: &[FileChange]| -> Result<LoadedConfig> {
        let mut edited = tree.clone();
        edited.apply(&file_ops(&tree, changes)?);
        edited.check_bounds()?;
        loaded(&edited)
    };
    let (changes, expected) = plan(&current, &tree.files, &preview)?;
    if changes.is_empty() {
        return Ok(current);
    }
    if !tree.writable && !dry_run {
        return Err(SaveError::ReadOnly.into());
    }
    let ops = file_ops(&tree, &changes)?;
    let mut edited = tree.clone();
    edited.apply(&ops);
    edited.check_bounds()?;
    let next = loaded(&edited)?;
    if next.config != expected {
        bail!("the edited files do not produce the planned configuration; nothing was written");
    }
    if dry_run {
        return Ok(next);
    }
    root.commit(&tree, &ops)?;
    loaded(&root.scan()?)
}

/// Checks whole-file changes against `tree` and turns them into operations.
fn file_ops(tree: &fs::Tree, changes: &[FileChange]) -> Result<Vec<fs::Op>> {
    let mut ops = Vec::new();
    for change in changes {
        let path = match change {
            FileChange::Write { path, .. } | FileChange::Delete { path } => path,
        };
        let parts: Vec<&str> = path.split('/').collect();
        if !path.ends_with(".yaml")
            || parts
                .iter()
                .any(|p| p.is_empty() || p.starts_with('.') || p.contains('\0'))
        {
            bail!("{path}: not a configuration file path");
        }
        if tree.dirs.contains(path) {
            bail!("{path}: is a directory");
        }
        match change {
            FileChange::Write { content, .. } => {
                for i in 1..parts.len() {
                    let dir = parts[..i].join("/");
                    let op = fs::Op::MkDir(dir.clone());
                    if !tree.dirs.contains(&dir) && !ops.contains(&op) {
                        ops.push(op);
                    }
                }
                ops.push(fs::Op::Write(path.clone(), content.clone()));
            }
            FileChange::Delete { .. } => {
                if !tree.files.contains_key(path) {
                    bail!("{path}: cannot delete a file that does not exist");
                }
                ops.push(fs::Op::Delete(path.clone()));
            }
        }
    }
    Ok(ops)
}

/// Renders a service as standalone YAML: the content of a
/// `machines/<machine>/services/<service>.yaml` file. Secret references
/// stay references.
pub fn export_service(service: &Service) -> Result<String> {
    let v = serde_json::to_value(service)?;
    Ok(save::document(&schema::shape(schema::Pos::Service, &v)))
}

/// Renders part of a root mapping, such as the content of [`LABELS_FILE`],
/// keeping only the keys it sets.
pub fn export_partial(value: &serde_json::Value) -> String {
    save::document(&schema::shape(schema::Pos::Root, value))
}

/// Renders a whole configuration as one root `_index.yaml`.
pub fn export_config(config: &Config) -> Result<String> {
    let v = serde_json::to_value(config)?;
    Ok(save::document(&schema::shape(schema::Pos::Root, &v)))
}
