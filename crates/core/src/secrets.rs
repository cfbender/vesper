//! Secret reference resolution.
//!
//! Configuration only ever stores [`SecretRef`] values (`{env: NAME}` or
//! `{file: filename}`); this module turns a reference into its value at the
//! moment a caller needs it. Resolved values are never written back into a
//! [`crate::Config`], and error messages name the reference but never include
//! any part of the secret content.

use crate::model::SecretRef;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;

/// Largest secret file accepted, in bytes.
pub const MAX_SECRET_FILE_BYTES: u64 = 64 * 1024;

/// Returns whether `name` is an acceptable environment variable name for a
/// secret reference: `[A-Za-z_][A-Za-z0-9_]*`, at most 128 bytes.
pub fn is_valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && name.len() <= 128
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Returns whether `name` is an acceptable secret file name: one path segment
/// of ASCII letters, digits, `_`, `-` and `.`, not starting with `.` or `-`,
/// at most 128 bytes. Subdirectories are deliberately not supported.
pub fn is_valid_file_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphanumeric() || b == b'_')
        && name.len() <= 128
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// Resolves a secret reference.
///
/// * `{env: NAME}` reads the process environment variable `NAME`.
/// * `{file: name}` reads `secrets_root/name`. The name must be a single safe
///   path segment; the file may be a symlink (as with Kubernetes secret
///   mounts) only if its canonical target stays inside the canonical
///   `secrets_root`. The file must be a regular file of at most
///   [`MAX_SECRET_FILE_BYTES`] bytes of UTF-8.
///
/// One trailing `\n` or `\r\n` is removed. Empty values are rejected.
pub fn resolve(secret: &SecretRef, secrets_root: &Path) -> Result<String> {
    let value = match secret {
        SecretRef::Env(name) => {
            if !is_valid_env_name(name) {
                bail!("secret reference has an invalid environment variable name");
            }
            match std::env::var(name) {
                Ok(v) => v,
                Err(std::env::VarError::NotPresent) => {
                    bail!("secret environment variable {name} is not set")
                }
                Err(std::env::VarError::NotUnicode(_)) => {
                    bail!("secret environment variable {name} is not valid UTF-8")
                }
            }
        }
        SecretRef::File(name) => read_secret_file(name, secrets_root)?,
    };
    let value = strip_one_newline(value);
    if value.is_empty() {
        bail!("secret {} is empty", describe(secret));
    }
    Ok(value)
}

/// A safe human-readable description of a reference (never its value).
pub fn describe(secret: &SecretRef) -> String {
    match secret {
        SecretRef::Env(name) if is_valid_env_name(name) => format!("env {name}"),
        SecretRef::File(name) if is_valid_file_name(name) => format!("file {name}"),
        SecretRef::Env(_) => "env <invalid name>".into(),
        SecretRef::File(_) => "file <invalid name>".into(),
    }
}

fn strip_one_newline(mut value: String) -> String {
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    value
}

fn read_secret_file(name: &str, secrets_root: &Path) -> Result<String> {
    if !is_valid_file_name(name) {
        bail!("secret reference has an invalid file name");
    }
    let root = secrets_root
        .canonicalize()
        .context("secrets directory is not accessible")?;
    let candidate = root.join(name);
    let target = candidate
        .canonicalize()
        .with_context(|| format!("secret file {name} is not accessible"))?;
    if !target.starts_with(&root) || target == root {
        bail!("secret file {name} resolves outside the secrets directory");
    }
    let mut file = std::fs::File::open(&target)
        .with_context(|| format!("secret file {name} cannot be opened"))?;
    let meta = file
        .metadata()
        .with_context(|| format!("secret file {name} cannot be inspected"))?;
    if !meta.is_file() {
        bail!("secret file {name} is not a regular file");
    }
    if meta.len() > MAX_SECRET_FILE_BYTES {
        bail!("secret file {name} exceeds {MAX_SECRET_FILE_BYTES} bytes");
    }
    let mut bytes = Vec::new();
    // Bound the read as well, in case the file grows after the metadata check.
    file.by_ref()
        .take(MAX_SECRET_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("secret file {name} cannot be read"))?;
    if bytes.len() as u64 > MAX_SECRET_FILE_BYTES {
        bail!("secret file {name} exceeds {MAX_SECRET_FILE_BYTES} bytes");
    }
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("secret file {name} is not valid UTF-8"))
}
