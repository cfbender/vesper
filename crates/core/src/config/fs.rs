//! Bounded, symlink-refusing reads of the configuration directory and
//! atomic writes back into it.
//!
//! Every access is descriptor-relative: the root directory is opened once
//! and each path component below it is opened with `O_NOFOLLOW` relative to
//! its already-opened parent, so replacing an ancestor directory with a
//! symlink cannot redirect reads or writes outside the root.

use super::SaveError;
use anyhow::{anyhow, bail, Context, Result};
use rustix::fs::{
    accessat, flock, fstat, fsync, mkdirat, openat, renameat, statat, unlinkat, Access, AtFlags,
    Dir, FileType, FlockOperation, Mode, OFlags,
};
use rustix::io::Errno;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;

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
    /// Whether every directory is writable by this process (writes replace
    /// files by rename, so directory permission is what matters).
    pub writable: bool,
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn split(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}

impl Tree {
    /// Immediate children of `dir`: (name, is_dir).
    pub fn children(&self, dir: &str) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        for d in &self.dirs {
            if !d.is_empty() && split(d).0 == dir {
                out.push((split(d).1.to_string(), true));
            }
        }
        for f in self.files.keys() {
            if split(f).0 == dir {
                out.push((split(f).1.to_string(), false));
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

/// A change to one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    MkDir(String),
    Write(String, String),
    Delete(String),
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn shown(rel: &str) -> &str {
    if rel.is_empty() {
        "configuration root"
    } else {
        rel
    }
}

fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

fn errno(e: Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(e.raw_os_error())
}

/// The opened configuration root. The root path itself may be a symlink
/// (for example a mount point); nothing below it may be.
pub struct Root {
    fd: OwnedFd,
}

/// Holds an advisory `flock` on the root directory, serializing Vesper's own
/// readers and writers. External editors do not take it.
pub struct Lock<'a>(&'a Root);

impl Drop for Lock<'_> {
    fn drop(&mut self) {
        let _ = flock(&self.0.fd, FlockOperation::Unlock);
    }
}

impl Root {
    pub fn open(root: &Path) -> Result<Root> {
        let canonical = root.canonicalize().with_context(|| {
            format!(
                "configuration directory {} is not accessible",
                root.display()
            )
        })?;
        let fd = rustix::fs::open(
            &canonical,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)
        .with_context(|| {
            format!(
                "configuration root {} is not a readable directory",
                root.display()
            )
        })?;
        Ok(Root { fd })
    }

    pub fn lock(&self, exclusive: bool) -> Result<Lock<'_>> {
        let op = if exclusive {
            FlockOperation::LockExclusive
        } else {
            FlockOperation::LockShared
        };
        flock(&self.fd, op)
            .map_err(errno)
            .context("cannot lock configuration directory")?;
        Ok(Lock(self))
    }

    /// Opens the directory `rel` one component at a time without following
    /// symlinks.
    fn dir(&self, rel: &str) -> Result<OwnedFd> {
        let mut fd = openat(&self.fd, ".", dir_flags(), Mode::empty()).map_err(errno)?;
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            fd = openat(&fd, part, dir_flags(), Mode::empty())
                .map_err(errno)
                .with_context(|| format!("{rel}: not a directory inside the configuration root"))?;
        }
        Ok(fd)
    }

    /// Reads the whole tree, refusing symlinks, special files, non-UTF-8
    /// names, non-`.yaml` files and anything beyond the size, count and
    /// depth bounds. Hidden entries (names starting with `.`) are ignored.
    pub fn scan(&self) -> Result<Tree> {
        let mut tree = Tree {
            writable: true,
            ..Tree::default()
        };
        tree.dirs.insert(String::new());
        let mut total = 0u64;
        let mut entries = 0usize;
        let mut stack = vec![(String::new(), self.dir("")?, 0usize)];
        while let Some((rel, fd, depth)) = stack.pop() {
            if accessat(&fd, ".", Access::WRITE_OK, AtFlags::empty()).is_err() {
                tree.writable = false;
            }
            let listing = Dir::read_from(&fd)
                .map_err(errno)
                .with_context(|| format!("cannot list {}", shown(&rel)))?;
            for item in listing {
                let item = item
                    .map_err(errno)
                    .with_context(|| format!("cannot list {}", shown(&rel)))?;
                let name = item
                    .file_name()
                    .to_str()
                    .map_err(|_| anyhow!("{}: file names must be UTF-8", shown(&rel)))?
                    .to_string();
                if name == "." || name == ".." || is_hidden(&name) {
                    continue;
                }
                entries += 1;
                if entries > MAX_ENTRIES {
                    bail!("configuration has more than {MAX_ENTRIES} entries");
                }
                let path = join(&rel, &name);
                let st = statat(&fd, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(errno)
                    .with_context(|| format!("cannot inspect {path}"))?;
                match FileType::from_raw_mode(st.st_mode) {
                    FileType::Symlink => {
                        bail!("{path}: symlinks are not allowed in the configuration directory")
                    }
                    FileType::Directory => {
                        if depth + 1 > MAX_DIR_DEPTH {
                            bail!("{path}: directories nest deeper than {MAX_DIR_DEPTH} levels");
                        }
                        let sub = openat(&fd, name.as_str(), dir_flags(), Mode::empty())
                            .map_err(errno)
                            .with_context(|| format!("cannot open {path}"))?;
                        tree.dirs.insert(path.clone());
                        stack.push((path, sub, depth + 1));
                    }
                    FileType::RegularFile => {
                        if name.ends_with(".yml") {
                            bail!("{path}: use the .yaml extension");
                        }
                        if !name.ends_with(".yaml") {
                            bail!("{path}: unexpected file; only .yaml files belong in the configuration directory (hidden files are ignored)");
                        }
                        let content = read_bounded(&fd, &name, &path)?;
                        total += content.len() as u64;
                        if total > MAX_TOTAL_BYTES {
                            bail!("configuration exceeds {MAX_TOTAL_BYTES} bytes in total");
                        }
                        tree.files.insert(path, content);
                    }
                    _ => bail!("{path}: only regular files and directories are allowed"),
                }
            }
        }
        Ok(tree)
    }

    /// Applies `ops`, which were planned against `base`.
    ///
    /// New directories are created and every write is staged into a hidden
    /// temporary sibling. The whole tree is then re-read; if anything differs
    /// from `base` (an external editor changed a file, even one this edit
    /// does not touch), the staged files are discarded and
    /// [`SaveError::Conflict`] is returned. Otherwise the staged files are
    /// renamed into place, deletions follow, and directories are fsynced.
    ///
    /// Each file replacement is atomic. A change made by a non-cooperating
    /// process between the re-read and the renames can still be overwritten,
    /// and a crash during a multi-file edit can leave some files updated.
    pub fn commit(&self, base: &Tree, ops: &[Op]) -> Result<()> {
        let mut created_dirs: Vec<String> = Vec::new();
        let mut staged: Vec<(OwnedFd, String, String)> = Vec::new();
        let result = self
            .stage(ops, &mut created_dirs, &mut staged)
            .and_then(|()| {
                let mut expected = base.clone();
                for d in &created_dirs {
                    expected.dirs.insert(d.clone());
                }
                let current = self.scan()?;
                if current.revision() != expected.revision() {
                    let mut now = current;
                    for d in &created_dirs {
                        now.dirs.remove(d);
                    }
                    return Err(SaveError::Conflict {
                        current_revision: now.revision(),
                    }
                    .into());
                }
                Ok(())
            });
        if let Err(e) = result {
            for (fd, tmp, _) in &staged {
                let _ = unlinkat(fd, tmp.as_str(), AtFlags::empty());
            }
            for d in created_dirs.iter().rev() {
                let (parent, name) = split(d);
                if let Ok(fd) = self.dir(parent) {
                    let _ = unlinkat(&fd, name, AtFlags::REMOVEDIR);
                }
            }
            return Err(e);
        }
        let mut synced: Vec<OwnedFd> = Vec::new();
        for (i, (fd, tmp, name)) in staged.iter().enumerate() {
            if let Err(e) = renameat(fd, tmp.as_str(), fd, name.as_str()) {
                for (fd, tmp, _) in &staged[i..] {
                    let _ = unlinkat(fd, tmp.as_str(), AtFlags::empty());
                }
                return Err(errno(e)).context("cannot replace configuration file");
            }
        }
        for op in ops {
            if let Op::Delete(rel) = op {
                let (parent, name) = split(rel);
                let fd = self.dir(parent)?;
                let st = statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(errno)
                    .with_context(|| format!("cannot inspect {rel}"))?;
                if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
                    bail!("{rel}: is not a regular file");
                }
                unlinkat(&fd, name, AtFlags::empty())
                    .map_err(errno)
                    .with_context(|| format!("cannot remove {rel}"))?;
                synced.push(fd);
            }
        }
        for (fd, _, _) in &staged {
            let _ = fsync(fd);
        }
        for fd in &synced {
            let _ = fsync(fd);
        }
        for d in &created_dirs {
            if let Ok(fd) = self.dir(split(d).0) {
                let _ = fsync(&fd);
            }
        }
        Ok(())
    }

    fn stage(
        &self,
        ops: &[Op],
        created_dirs: &mut Vec<String>,
        staged: &mut Vec<(OwnedFd, String, String)>,
    ) -> Result<()> {
        for op in ops {
            if let Op::MkDir(d) = op {
                let (parent, name) = split(d);
                let fd = self.dir(parent)?;
                match mkdirat(&fd, name, Mode::from_raw_mode(0o755)) {
                    Ok(()) => created_dirs.push(d.clone()),
                    Err(Errno::EXIST) => {
                        self.dir(d)?;
                    }
                    Err(e) => return Err(errno(e)).with_context(|| format!("cannot create {d}")),
                }
            }
        }
        for op in ops {
            if let Op::Write(rel, content) = op {
                let (parent, name) = split(rel);
                let fd = self.dir(parent)?;
                let mode = match statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(st) if FileType::from_raw_mode(st.st_mode) == FileType::RegularFile => {
                        st.st_mode & 0o777
                    }
                    Ok(_) => bail!("{rel}: is not a regular file"),
                    Err(_) => 0o644,
                };
                let tmp = format!(".{name}.vesper-{}-{}.tmp", std::process::id(), unique());
                let file = openat(
                    &fd,
                    tmp.as_str(),
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::from_raw_mode(mode),
                )
                .map_err(errno)
                .with_context(|| format!("cannot write next to {rel}"))?;
                let mut file = File::from(file);
                staged.push((fd, tmp, name.to_string()));
                file.write_all(content.as_bytes())
                    .and_then(|_| file.sync_all())
                    .with_context(|| format!("cannot write {rel}"))?;
            }
        }
        Ok(())
    }
}

fn read_bounded(dir: &OwnedFd, name: &str, rel: &str) -> Result<String> {
    let fd = openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(errno)
    .with_context(|| format!("cannot open {rel}"))?;
    let st = fstat(fd.as_fd())
        .map_err(errno)
        .with_context(|| format!("cannot inspect {rel}"))?;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
        bail!("{rel}: changed while being read");
    }
    if st.st_size as u64 > MAX_FILE_BYTES {
        bail!("{rel}: larger than {MAX_FILE_BYTES} bytes");
    }
    let mut bytes = Vec::new();
    File::from(fd)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {rel}"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!("{rel}: larger than {MAX_FILE_BYTES} bytes");
    }
    String::from_utf8(bytes).map_err(|_| anyhow!("{rel}: not valid UTF-8"))
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as u64);
    (nanos << 20) ^ N.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (tempfile::TempDir, Root, Tree) {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("machines")).unwrap();
        fs::write(d.path().join("machines/a.yaml"), "name: A\n").unwrap();
        fs::write(d.path().join("_index.yaml"), "{}\n").unwrap();
        let root = Root::open(d.path()).unwrap();
        let tree = root.scan().unwrap();
        (d, root, tree)
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn external_change_to_any_file_aborts_commit() {
        let (d, root, tree) = setup();
        // An editor changes a file the edit does not touch.
        fs::write(d.path().join("_index.yaml"), "{} # edited\n").unwrap();
        let ops = vec![
            Op::MkDir("machines/b".into()),
            Op::Write("machines/a.yaml".into(), "name: A2\n".into()),
            Op::Write("machines/b/_index.yaml".into(), "name: B\n".into()),
        ];
        let e = root.commit(&tree, &ops).unwrap_err();
        let current = root.scan().unwrap().revision();
        assert_eq!(
            e.downcast_ref::<SaveError>(),
            Some(&SaveError::Conflict {
                current_revision: current
            })
        );
        assert_eq!(
            fs::read_to_string(d.path().join("machines/a.yaml")).unwrap(),
            "name: A\n"
        );
        assert_eq!(
            names(&d.path().join("machines")),
            vec!["a.yaml"],
            "staged files and new dirs removed"
        );
    }

    #[test]
    fn ancestor_swapped_for_symlink_cannot_redirect_writes() {
        let (d, root, tree) = setup();
        let outside = tempfile::tempdir().unwrap();
        fs::rename(d.path().join("machines"), d.path().join(".old")).unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("machines")).unwrap();
        let ops = vec![Op::Write("machines/a.yaml".into(), "pwned\n".into())];
        assert!(root.commit(&tree, &ops).is_err());
        assert!(
            names(outside.path()).is_empty(),
            "nothing written outside the root"
        );
        assert!(root
            .scan()
            .unwrap_err()
            .to_string()
            .contains("symlinks are not allowed"));
    }

    #[test]
    fn commit_applies_writes_and_deletes() {
        let (d, root, tree) = setup();
        let ops = vec![
            Op::MkDir("machines/b".into()),
            Op::Write("machines/b/_index.yaml".into(), "name: B\n".into()),
            Op::Delete("machines/a.yaml".into()),
        ];
        root.commit(&tree, &ops).unwrap();
        let mut expected = tree.clone();
        expected.apply(&ops);
        assert_eq!(root.scan().unwrap().revision(), expected.revision());
        assert_eq!(names(&d.path().join("machines")), vec!["b"]);
    }
}
