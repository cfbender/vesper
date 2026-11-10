//! Bounded, symlink-refusing reads of the configuration directory and
//! atomic writes back into it.

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 10_000;
/// Directory nesting below the root (`machines/m/services/s/checks` is 5).
pub const MAX_DIR_DEPTH: usize = 6;

/// An in-memory image of the configuration tree. Paths are root-relative,
/// `/`-separated, without leading or trailing slashes; `""` is the root.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub dirs: BTreeSet<String>,
    pub files: BTreeMap<String, String>,
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

impl Tree {
    /// Immediate children of `dir`: (name, is_dir).
    pub fn children(&self, dir: &str) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        for d in &self.dirs {
            if !d.is_empty() && parent(d) == dir {
                out.push((d[d.rfind('/').map_or(0, |i| i + 1)..].to_string(), true));
            }
        }
        for f in self.files.keys() {
            if parent(f) == dir {
                out.push((f[f.rfind('/').map_or(0, |i| i + 1)..].to_string(), false));
            }
        }
        out.sort();
        out
    }

    pub fn revision(&self) -> String {
        let mut h = Sha256::new();
        for d in &self.dirs {
            h.update(b"D\0");
            h.update(d.as_bytes());
            h.update(b"\0");
        }
        for (path, content) in &self.files {
            h.update(b"F\0");
            h.update(path.as_bytes());
            h.update(b"\0");
            h.update((content.len() as u64).to_le_bytes());
            h.update(content.as_bytes());
        }
        let digest = h.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256:{hex}")
    }
}

/// Canonical configuration root. The root path itself may be a symlink (for
/// example a mount point); nothing below it may be.
pub fn root_dir(root: &Path) -> Result<PathBuf> {
    let canonical = root.canonicalize().with_context(|| {
        format!(
            "configuration directory {} is not accessible",
            root.display()
        )
    })?;
    if !canonical.is_dir() {
        bail!("configuration root {} is not a directory", root.display());
    }
    Ok(canonical)
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Reads the whole tree, refusing symlinks, special files, non-UTF-8 names,
/// non-`.yaml` files and anything beyond the size, count and depth bounds.
/// Hidden entries (names starting with `.`) are ignored.
pub fn scan(root: &Path) -> Result<Tree> {
    let mut tree = Tree::default();
    tree.dirs.insert(String::new());
    let mut total = 0u64;
    let mut entries = 0usize;
    let mut stack = vec![(String::new(), 0usize)];
    while let Some((rel, depth)) = stack.pop() {
        let dir = root.join(&rel);
        let iter = fs::read_dir(&dir).with_context(|| format!("cannot list {}", display(&rel)))?;
        for item in iter {
            let item = item.with_context(|| format!("cannot list {}", display(&rel)))?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| anyhow!("{}: file names must be UTF-8", display(&rel)))?;
            if is_hidden(&name) {
                continue;
            }
            entries += 1;
            if entries > MAX_ENTRIES {
                bail!("configuration has more than {MAX_ENTRIES} entries");
            }
            let path = join(&rel, &name);
            let meta = fs::symlink_metadata(item.path())
                .with_context(|| format!("cannot inspect {path}"))?;
            let ft = meta.file_type();
            if ft.is_symlink() {
                bail!("{path}: symlinks are not allowed in the configuration directory");
            } else if ft.is_dir() {
                if depth + 1 > MAX_DIR_DEPTH {
                    bail!("{path}: directories nest deeper than {MAX_DIR_DEPTH} levels");
                }
                tree.dirs.insert(path.clone());
                stack.push((path, depth + 1));
            } else if ft.is_file() {
                if name.ends_with(".yml") {
                    bail!("{path}: use the .yaml extension");
                }
                if !name.ends_with(".yaml") {
                    bail!("{path}: unexpected file; only .yaml files belong in the configuration directory (hidden files are ignored)");
                }
                let content = read_bounded(&item.path(), &path, meta.dev(), meta.ino())?;
                total += content.len() as u64;
                if total > MAX_TOTAL_BYTES {
                    bail!("configuration exceeds {MAX_TOTAL_BYTES} bytes in total");
                }
                tree.files.insert(path, content);
            } else {
                bail!("{path}: only regular files and directories are allowed");
            }
        }
    }
    Ok(tree)
}

fn display(rel: &str) -> &str {
    if rel.is_empty() {
        "configuration root"
    } else {
        rel
    }
}

fn read_bounded(path: &Path, rel: &str, dev: u64, ino: u64) -> Result<String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("cannot open {rel}"))?;
    let meta = file
        .metadata()
        .with_context(|| format!("cannot inspect {rel}"))?;
    if !meta.is_file() || meta.dev() != dev || meta.ino() != ino {
        bail!("{rel}: changed while being read");
    }
    if meta.len() > MAX_FILE_BYTES {
        bail!("{rel}: larger than {MAX_FILE_BYTES} bytes");
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {rel}"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!("{rel}: larger than {MAX_FILE_BYTES} bytes");
    }
    String::from_utf8(bytes).map_err(|_| anyhow!("{rel}: not valid UTF-8"))
}

fn access_writable(path: &Path) -> bool {
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Whether every directory of the tree is writable by this process (writes
/// replace files by rename, so directory permission is what matters).
pub fn writable(root: &Path, tree: &Tree) -> bool {
    tree.dirs.iter().all(|d| access_writable(&root.join(d)))
}

/// An exclusive advisory lock on the root directory, serializing Vesper's
/// own writers. External editors do not take it.
pub struct RootLock(File);

impl RootLock {
    pub fn acquire(root: &Path, exclusive: bool) -> Result<RootLock> {
        let f = File::open(root).context("cannot open configuration directory")?;
        let mode = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        // SAFETY: flock on a valid, owned descriptor.
        if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&f), mode) } != 0 {
            bail!("cannot lock configuration directory");
        }
        Ok(RootLock(f))
    }
}

impl Drop for RootLock {
    fn drop(&mut self) {
        // SAFETY: unlocking the descriptor locked in `acquire`.
        unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&self.0), libc::LOCK_UN) };
    }
}

/// A change to one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    MkDir(String),
    Write(String, String),
    Delete(String),
}

impl Tree {
    pub fn apply(&mut self, ops: &[Op]) {
        for op in ops {
            match op {
                Op::MkDir(d) => {
                    self.dirs.insert(d.clone());
                }
                Op::Write(p, c) => {
                    self.files.insert(p.clone(), c.clone());
                }
                Op::Delete(p) => {
                    self.files.remove(p);
                }
            }
        }
    }
}

/// Writes changes: new directories, then every file to a temporary sibling,
/// then renames them into place, then deletions, then directory fsyncs.
/// Each file replacement is atomic; a crash between renames of a
/// multi-file edit can leave some files updated and others not.
pub fn commit(root: &Path, ops: &[Op]) -> Result<()> {
    let mut touched_dirs = BTreeSet::new();
    for op in ops {
        if let Op::MkDir(d) = op {
            let path = root.join(d);
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let meta = fs::symlink_metadata(&path)?;
                    if !meta.is_dir() {
                        bail!("{d}: exists and is not a directory");
                    }
                }
                Err(e) => return Err(e).with_context(|| format!("cannot create {d}")),
            }
            touched_dirs.insert(parent(d).to_string());
        }
    }
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = (|| -> Result<()> {
        for op in ops {
            if let Op::Write(rel, content) = op {
                let target = root.join(rel);
                let mut mode = 0o644;
                if let Ok(meta) = fs::symlink_metadata(&target) {
                    if !meta.is_file() {
                        bail!("{rel}: is not a regular file");
                    }
                    mode = meta.mode() & 0o777;
                }
                let name = rel.rsplit('/').next().unwrap_or(rel);
                let tmp = target.with_file_name(format!(
                    ".{name}.vesper-{}-{}.tmp",
                    std::process::id(),
                    unique()
                ));
                let mut f = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(&tmp)
                    .with_context(|| format!("cannot write next to {rel}"))?;
                staged.push((tmp, target));
                f.write_all(content.as_bytes())
                    .and_then(|_| f.sync_all())
                    .with_context(|| format!("cannot write {rel}"))?;
                touched_dirs.insert(parent(rel).to_string());
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        for (tmp, _) in &staged {
            let _ = fs::remove_file(tmp);
        }
        return Err(e);
    }
    for (i, (tmp, target)) in staged.iter().enumerate() {
        if let Err(e) = fs::rename(tmp, target) {
            for (tmp, _) in &staged[i..] {
                let _ = fs::remove_file(tmp);
            }
            return Err(e).context("cannot replace configuration file");
        }
    }
    for op in ops {
        if let Op::Delete(rel) = op {
            let path = root.join(rel);
            let meta =
                fs::symlink_metadata(&path).with_context(|| format!("cannot inspect {rel}"))?;
            if !meta.is_file() {
                bail!("{rel}: is not a regular file");
            }
            fs::remove_file(&path).with_context(|| format!("cannot remove {rel}"))?;
            touched_dirs.insert(parent(rel).to_string());
        }
    }
    for d in touched_dirs {
        if let Ok(f) = File::open(root.join(&d)) {
            let _ = f.sync_all();
        }
    }
    Ok(())
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as u64);
    (nanos << 20) ^ N.fetch_add(1, Ordering::Relaxed)
}
