//! Turns a [`Tree`] into one configuration value, enforcing the directory
//! layout rules and recording where every value is defined.

use super::fs::{join, Tree};
use super::schema::Pos;
use super::yaml::{self, safe_key, Doc, Entry, Kind, Node};
use crate::model::Config;
use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const INDEX: &str = "_index.yaml";

#[derive(Debug, Clone)]
pub struct DirNode {
    pub path: String,
    /// Root-relative path of this directory's `_index.yaml`, when present.
    pub index: Option<String>,
    pub entries: BTreeMap<String, DirEntry>,
}

#[derive(Debug, Clone)]
pub enum DirEntry {
    /// Defined as a key of `_index.yaml`.
    Index,
    /// Defined by the whole file at this root-relative path.
    File(String),
    Dir(DirNode),
}

/// The assembled tree with every parsed file.
#[derive(Debug, Clone)]
pub struct Assembled {
    pub docs: BTreeMap<String, Doc>,
    pub root: DirNode,
    pub value: Value,
}

/// Where a value is defined: inside a YAML file or as a directory.
#[derive(Debug, Clone, Copy)]
pub enum Raw<'a> {
    Yaml {
        file: &'a str,
        /// `None` for an empty document.
        node: Option<&'a Node>,
        slot: Slot<'a>,
    },
    Dir(&'a DirNode),
}

/// How a YAML value is attached: as a whole document or as a mapping value.
#[derive(Debug, Clone, Copy)]
pub enum Slot<'a> {
    Root,
    /// A value in a block mapping.
    Entry(&'a Entry),
    /// A value in a flow mapping (`{a: 1}`).
    FlowEntry(&'a Entry),
}

fn entry_slot<'a>(map: &Node, e: &'a Entry) -> Slot<'a> {
    if map.flow {
        Slot::FlowEntry(e)
    } else {
        Slot::Entry(e)
    }
}

impl Assembled {
    pub fn root(&self) -> Raw<'_> {
        Raw::Dir(&self.root)
    }

    pub fn doc(&self, file: &str) -> &Doc {
        &self.docs[file]
    }

    pub fn children<'a>(&'a self, raw: Raw<'a>) -> Vec<(&'a str, Raw<'a>)> {
        match raw {
            Raw::Yaml {
                file,
                node: Some(node),
                ..
            } => match &node.kind {
                Kind::Map(entries) => entries
                    .iter()
                    .map(|e| {
                        (
                            e.key.as_str(),
                            Raw::Yaml {
                                file,
                                node: Some(&e.value),
                                slot: entry_slot(node, e),
                            },
                        )
                    })
                    .collect(),
                _ => vec![],
            },
            Raw::Yaml { node: None, .. } => vec![],
            Raw::Dir(d) => d
                .entries
                .iter()
                .map(|(k, entry)| {
                    let raw = match entry {
                        DirEntry::Index => {
                            let file = d.index.as_deref().expect("index entry without index");
                            let doc = &self.docs[file];
                            let entry = index_entries(doc)
                                .iter()
                                .find(|e| &e.key == k)
                                .expect("index entry");
                            Raw::Yaml {
                                file,
                                node: Some(&entry.value),
                                slot: entry_slot(doc.root.as_ref().expect("index root"), entry),
                            }
                        }
                        DirEntry::File(f) => Raw::Yaml {
                            file: f,
                            node: self.docs[f].root.as_ref(),
                            slot: Slot::Root,
                        },
                        DirEntry::Dir(sub) => Raw::Dir(sub),
                    };
                    (k.as_str(), raw)
                })
                .collect(),
        }
    }

    pub fn child<'a>(&'a self, raw: Raw<'a>, key: &str) -> Option<Raw<'a>> {
        self.children(raw)
            .into_iter()
            .find_map(|(k, r)| (k == key).then_some(r))
    }

    /// The file a logical node is reported as coming from.
    pub fn source_of(raw: Raw<'_>) -> String {
        match raw {
            Raw::Yaml { file, .. } => file.to_string(),
            Raw::Dir(d) => match &d.index {
                Some(i) => i.clone(),
                None => format!("{}/", d.path),
            },
        }
    }

    /// Logical path to defining file for machines, services, checks and the
    /// top-level `defaults`, `network`, `telegram` and `notifications` sections.
    pub fn sources(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        self.collect_sources(self.root(), Pos::Root, "", &mut out);
        out
    }

    fn collect_sources(
        &self,
        raw: Raw<'_>,
        pos: Pos,
        logical: &str,
        out: &mut BTreeMap<String, String>,
    ) {
        for (key, child) in self.children(raw) {
            let child_pos = pos.child(key);
            let path = join(logical, key);
            let record = matches!(child_pos, Pos::Machine | Pos::Service | Pos::Check)
                || (pos == Pos::Root
                    && matches!(key, "defaults" | "network" | "telegram" | "notifications"));
            if record {
                out.insert(path.clone(), Self::source_of(child));
            }
            if child_pos.is_collection() || matches!(child_pos, Pos::Machine | Pos::Service) {
                self.collect_sources(child, child_pos, &path, out);
            }
        }
    }
}

fn index_entries(doc: &Doc) -> &[Entry] {
    match doc.root.as_ref().map(|n| &n.kind) {
        Some(Kind::Map(entries)) => entries,
        _ => &[],
    }
}

/// Assembles a tree into raw structure plus a typed configuration.
pub fn assemble(tree: &Tree) -> Result<(Assembled, Config)> {
    let mut docs = BTreeMap::new();
    let (root, value) = assemble_dir(tree, &mut docs, "", Pos::Root)?;
    let assembled = Assembled { docs, root, value };
    let config = deserialize(&assembled)?;
    Ok((assembled, config))
}

fn parse_file(tree: &Tree, docs: &mut BTreeMap<String, Doc>, path: &str) -> Result<()> {
    let doc = yaml::parse(&tree.files[path]).map_err(|e| anyhow!("{path}: {e}"))?;
    docs.insert(path.to_string(), doc);
    Ok(())
}

pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= super::MAX_ID_BYTES
        && id != "_index"
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn assemble_dir(
    tree: &Tree,
    docs: &mut BTreeMap<String, Doc>,
    path: &str,
    pos: Pos,
) -> Result<(DirNode, Value)> {
    let shown = if path.is_empty() {
        "configuration root".to_string()
    } else {
        format!("{path}/")
    };
    let mut node = DirNode {
        path: path.to_string(),
        index: None,
        entries: BTreeMap::new(),
    };
    let mut obj = Map::new();
    let children = tree.children(path);
    if children.iter().any(|(n, d)| n == INDEX && !d) {
        let file = join(path, INDEX);
        parse_file(tree, docs, &file)?;
        let doc = &docs[&file];
        match doc.root.as_ref().map(|n| &n.kind) {
            None | Some(Kind::Map(_)) => {}
            Some(_) => bail!("{file}: must contain a mapping"),
        }
        for e in index_entries(doc) {
            obj.insert(e.key.clone(), yaml::to_json(&e.value));
            node.entries.insert(e.key.clone(), DirEntry::Index);
        }
        node.index = Some(file);
    }
    for (name, is_dir) in &children {
        if name == INDEX && !is_dir {
            continue;
        }
        let rel = join(path, name);
        let key = if *is_dir {
            name.as_str()
        } else {
            name.strip_suffix(".yaml")
                .expect("scan admits only .yaml files")
        };
        let what = if *is_dir {
            format!("{rel}/")
        } else {
            rel.clone()
        };
        if pos.is_collection() {
            if !is_valid_id(key) {
                bail!("{what}: name is not a valid ID (ASCII letters, digits, '-' and '_', at most {} bytes)", super::MAX_ID_BYTES);
            }
        } else if !pos.split_fields().contains(&key) {
            let allowed = pos.split_fields();
            if allowed.is_empty() {
                bail!("{what}: unexpected entry in {shown}");
            }
            bail!(
                "{what}: unexpected entry in {shown}; only {} may be split into files, other fields belong in {}",
                allowed.join(", "),
                join(path, INDEX)
            );
        }
        let child_pos = pos.child(key);
        if *is_dir && !child_pos.allows_dir() {
            bail!(
                "{what}: cannot be a directory; use {}.yaml",
                join(path, key)
            );
        }
        if !is_dir && children.iter().any(|(n, d)| *d && n == key) {
            bail!("{rel} and {}/ both define `{key}`", join(path, key));
        }
        if node.entries.contains_key(key) {
            bail!(
                "`{key}` is defined both in {} and by {what}",
                join(path, INDEX)
            );
        }
        let (entry, value) = if *is_dir {
            let (sub, value) = assemble_dir(tree, docs, &rel, child_pos)?;
            (DirEntry::Dir(sub), value)
        } else {
            parse_file(tree, docs, &rel)?;
            let value = match &docs[&rel].root {
                None if child_pos.is_mapping() => Value::Object(Map::new()),
                None => Value::Null,
                Some(n) => yaml::to_json(n),
            };
            (DirEntry::File(rel.clone()), value)
        };
        node.entries.insert(key.to_string(), entry);
        obj.insert(key.to_string(), value);
    }
    Ok((node, Value::Object(obj)))
}

fn deserialize(a: &Assembled) -> Result<Config> {
    serde_path_to_error::deserialize::<_, Config>(a.value.clone()).map_err(|e| {
        let path = e.path().to_string();
        let segments: Vec<&str> = path
            .split('.')
            .filter(|s| !s.is_empty() && *s != "?")
            .collect();
        let logical = segments.join("/");
        let sources = a.sources();
        let mut file = None;
        let mut probe = logical.as_str();
        loop {
            if let Some(f) = sources.get(probe) {
                file = Some(f.clone());
                break;
            }
            match probe.rfind('/') {
                Some(i) => probe = &probe[..i],
                None => break,
            }
        }
        let file = file.unwrap_or_else(|| locate_top(a, segments.first().copied()));
        let shown_path: Vec<&str> = segments.iter().map(|s| safe_key(s)).collect();
        anyhow!(
            "{file}: {}: {}",
            if shown_path.is_empty() {
                "configuration".to_string()
            } else {
                shown_path.join(".")
            },
            sanitize_serde(&e.into_inner().to_string())
        )
    })
}

fn locate_top(a: &Assembled, key: Option<&str>) -> String {
    key.and_then(|k| a.child(a.root(), k))
        .map(Assembled::source_of)
        .or_else(|| a.root.index.clone())
        .unwrap_or_else(|| "configuration root".into())
}

/// Removes quoted values from serde messages, which might be misplaced
/// secret material, keeping field names and the expectation.
pub fn sanitize_serde(msg: &str) -> String {
    let (head, tail) = match msg.find("expected") {
        Some(i) => msg.split_at(i),
        None => (msg, ""),
    };
    let keep = head.starts_with("unknown field") || head.starts_with("missing field");
    let mut out = String::new();
    let mut quote: Option<char> = None;
    for c in head.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                out.push(c);
            }
            Some(_) if keep => out.push(c),
            Some(_) => {}
            None if c == '"' || c == '`' => {
                quote = Some(c);
                out.push(c);
                if !keep {
                    out.push('…');
                }
            }
            None => out.push(c),
        }
    }
    // Truncate an unterminated quote and remove line/column noise from serde_json.
    let tail = tail.split(" at line ").next().unwrap_or(tail);
    format!("{out}{tail}")
}
