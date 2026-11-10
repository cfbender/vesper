//! Turns a typed configuration change into minimal edits of the files that
//! define it: changed scalars are replaced in place, removed keys lose
//! their lines, new keys are appended to their mapping, and new entries of
//! directory-backed collections become new files. Text outside changed
//! values, including comments, is preserved byte for byte. Anything that
//! would discard comments or needs a layout change becomes
//! [`SaveError::ExportOnly`].

use super::assemble::{Assembled, DirEntry, DirNode, Raw, Slot, INDEX};
use super::fs::{join, Op};
use super::schema::{shape, Pos};
use super::yaml::{
    self, emit_after_key, emit_block, emit_flow_seq, emit_str, line_end, line_start, Kind, Node,
    Out,
};
use super::SaveError;
use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
struct TextEdit {
    start: usize,
    end: usize,
    text: String,
}

struct Planner<'a> {
    a: &'a Assembled,
    edits: BTreeMap<String, Vec<TextEdit>>,
    dirs: Vec<String>,
    /// Files to create, as ordered top-level entries.
    new_files: BTreeMap<String, Vec<(String, Out)>>,
    whole: BTreeMap<String, String>,
    deletes: Vec<String>,
}

fn export_only(reason: impl Into<String>) -> anyhow::Error {
    SaveError::ExportOnly {
        reason: reason.into(),
    }
    .into()
}

static NULL: Value = Value::Null;

fn get<'v>(v: &'v Value, key: &str) -> &'v Value {
    v.get(key).unwrap_or(&NULL)
}

/// Plans file operations turning `old` (the configuration `a` defines) into `new`.
pub fn plan(a: &Assembled, old: &Value, new: &Value) -> Result<Vec<Op>> {
    let mut p = Planner {
        a,
        edits: BTreeMap::new(),
        dirs: Vec::new(),
        new_files: BTreeMap::new(),
        whole: BTreeMap::new(),
        deletes: Vec::new(),
    };
    p.diff(a.root(), Pos::Root, old, new)?;
    p.finish()
}

pub fn document(out: &Out) -> String {
    match out {
        Out::Map(m) if m.is_empty() => "{}\n".into(),
        other => {
            let mut s = String::new();
            emit_block(other, 0, &mut s);
            s
        }
    }
}

impl<'a> Planner<'a> {
    fn diff(&mut self, raw: Raw<'a>, pos: Pos, old: &Value, new: &Value) -> Result<()> {
        if old == new {
            return Ok(());
        }
        match raw {
            Raw::Dir(d) => self.diff_dir(d, pos, old, new),
            Raw::Yaml { file, node, slot } => match node {
                Some(n) if matches!(n.kind, Kind::Map(_)) && !n.flow && new.is_object() => {
                    self.diff_map(file, n, slot, pos, old, new)
                }
                Some(n) => self.replace(file, n, slot, pos, new, false),
                None => self.fill_empty_doc(file, pos, new),
            },
        }
    }

    fn diff_dir(&mut self, d: &'a DirNode, pos: Pos, old: &Value, new: &Value) -> Result<()> {
        let Some(new_o) = new.as_object() else {
            return Err(export_only(format!(
                "{}/ would no longer be a mapping",
                d.path
            )));
        };
        let children: BTreeMap<&str, Raw<'a>> = self.a.children(Raw::Dir(d)).into_iter().collect();
        for (&key, &raw) in &children {
            let (o, n) = (get(old, key), get(new, key));
            if o == n {
                continue;
            }
            if n.is_null() {
                match &d.entries[key] {
                    DirEntry::Index => {
                        let Raw::Yaml {
                            file,
                            slot: Slot::Entry(e),
                            ..
                        } = raw
                        else {
                            unreachable!()
                        };
                        self.delete_entry(file, e)?;
                    }
                    DirEntry::File(f) => self.deletes.push(f.clone()),
                    DirEntry::Dir(sub) => {
                        return Err(export_only(format!(
                            "removing directory {}/ is not supported",
                            sub.path
                        )))
                    }
                }
            } else {
                self.diff(raw, pos.child(key), o, n)?;
            }
        }
        for key in pos.ordered_keys(new_o.keys().filter(|k| !children.contains_key(k.as_str()))) {
            let n = &new_o[&key];
            if get(old, &key) == n || n.is_null() || !pos.is_collection() && pos.omit(&key, n) {
                continue;
            }
            self.insert_dir_entry(d, pos, &key, n)?;
        }
        Ok(())
    }

    fn insert_dir_entry(&mut self, d: &DirNode, pos: Pos, key: &str, n: &Value) -> Result<()> {
        let child = pos.child(key);
        if pos.is_collection() {
            let siblings_are_dirs = d.entries.values().any(|e| matches!(e, DirEntry::Dir(_)));
            if child.allows_dir() && siblings_are_dirs {
                self.new_dir(&join(&d.path, key), child, n);
            } else {
                self.new_file(&join(&d.path, &format!("{key}.yaml")), child, n);
            }
        } else if child.is_collection() && pos.split_fields().contains(&key) {
            self.new_dir(&join(&d.path, key), child, n);
        } else {
            match &d.index {
                Some(file) => {
                    let doc = self.a.doc(file);
                    match &doc.root {
                        Some(root) if matches!(root.kind, Kind::Map(_)) && !root.flow => self
                            .insert_entries(file, root, vec![(key.to_string(), shape(child, n))])?,
                        None => {
                            self.append_to_empty(file, vec![(key.to_string(), shape(child, n))])
                        }
                        Some(_) => {
                            return Err(export_only(format!("{file} is not a block mapping")))
                        }
                    }
                }
                None => self
                    .new_files
                    .entry(join(&d.path, INDEX))
                    .or_default()
                    .push((key.to_string(), shape(child, n))),
            }
        }
        Ok(())
    }

    /// A new directory-backed node: collections get one file per entry;
    /// structs get `_index.yaml` plus directories for split collections.
    fn new_dir(&mut self, path: &str, pos: Pos, v: &Value) {
        self.dirs.push(path.to_string());
        let empty = Map::new();
        let o = v.as_object().unwrap_or(&empty);
        if pos.is_collection() {
            for (k, child) in o {
                self.new_file(&join(path, &format!("{k}.yaml")), pos.child(k), child);
            }
            return;
        }
        let mut index = Vec::new();
        for k in pos.ordered_keys(o.keys()) {
            let child = &o[&k];
            let cp = pos.child(&k);
            if pos.omit(&k, child) {
                continue;
            }
            if cp.is_collection() && pos.split_fields().contains(&k.as_str()) {
                if child.as_object().is_some_and(|m| !m.is_empty()) {
                    self.new_dir(&join(path, &k), cp, child);
                }
            } else {
                index.push((k.clone(), shape(cp, child)));
            }
        }
        if !index.is_empty() {
            self.new_files.insert(join(path, INDEX), index);
        }
    }

    fn new_file(&mut self, path: &str, pos: Pos, v: &Value) {
        self.whole
            .insert(path.to_string(), document(&shape(pos, v)));
    }

    fn diff_map(
        &mut self,
        file: &'a str,
        map: &'a Node,
        slot: Slot<'a>,
        pos: Pos,
        old: &Value,
        new: &Value,
    ) -> Result<()> {
        let Kind::Map(entries) = &map.kind else {
            unreachable!()
        };
        let new_o = new.as_object().expect("checked by caller");
        let removed: Vec<_> = entries
            .iter()
            .filter(|e| get(old, &e.key) != get(new, &e.key) && get(new, &e.key).is_null())
            .collect();
        let inserts: Vec<(String, Out)> = pos
            .ordered_keys(
                new_o
                    .keys()
                    .filter(|k| !entries.iter().any(|e| &e.key == *k)),
            )
            .into_iter()
            .filter(|k| {
                let n = &new_o[k];
                !(get(old, k) == n || n.is_null() || !pos.is_collection() && pos.omit(k, n))
            })
            .map(|k| {
                let out = shape(pos.child(&k), &new_o[&k]);
                (k, out)
            })
            .collect();
        if removed.len() == entries.len() && inserts.is_empty() {
            // Everything goes. A document with no content is an empty
            // mapping; a nested mapping is replaced by `{}`.
            return match slot {
                Slot::Root => {
                    for e in removed {
                        self.delete_entry(file, e)?;
                    }
                    Ok(())
                }
                Slot::Entry(_) => self.replace(file, map, slot, pos, new, true),
            };
        }
        for e in entries {
            let (o, n) = (get(old, &e.key), get(new, &e.key));
            if o == n {
                continue;
            }
            if n.is_null() {
                self.delete_entry(file, e)?;
            } else {
                let raw = Raw::Yaml {
                    file,
                    node: Some(&e.value),
                    slot: Slot::Entry(e),
                };
                self.diff(raw, pos.child(&e.key), o, n)?;
            }
        }
        if !inserts.is_empty() {
            self.insert_entries(file, map, inserts)?;
        }
        Ok(())
    }

    fn key_indent(&self, file: &str, key_start: usize) -> Result<usize> {
        let text = &self.a.doc(file).text;
        let ls = line_start(text, key_start);
        if text[ls..key_start].bytes().all(|b| b == b' ') {
            Ok(key_start - ls)
        } else {
            Err(export_only(format!(
                "{file}: unusual key layout near line {}",
                1 + text[..key_start].matches('\n').count()
            )))
        }
    }

    fn push_edit(&mut self, file: &str, start: usize, end: usize, text: String) {
        self.edits
            .entry(file.to_string())
            .or_default()
            .push(TextEdit { start, end, text });
    }

    fn delete_entry(&mut self, file: &str, e: &yaml::Entry) -> Result<()> {
        self.key_indent(file, e.key_start)?;
        let text = &self.a.doc(file).text;
        let start = line_start(text, e.key_start);
        let end_pos = e.value.end.max(e.key_end);
        let end = line_end(text, end_pos);
        let rest = text[end_pos..end].trim_start();
        if !(rest.is_empty() || rest.starts_with('#')) {
            return Err(export_only(format!(
                "{file}: unusual layout after `{}`",
                yaml::safe_key(&e.key)
            )));
        }
        self.push_edit(file, start, end, String::new());
        Ok(())
    }

    fn insert_entries(
        &mut self,
        file: &str,
        map: &Node,
        inserts: Vec<(String, Out)>,
    ) -> Result<()> {
        let Kind::Map(entries) = &map.kind else {
            unreachable!()
        };
        let first = entries.first().expect("block mappings are never empty");
        let indent = self.key_indent(file, first.key_start)?;
        let text = &self.a.doc(file).text;
        let at = line_end(text, map.end);
        let mut out = String::new();
        if at == text.len() && !text.ends_with('\n') {
            out.push('\n');
        }
        for (k, v) in inserts {
            out.push_str(&yaml::indent(indent));
            out.push_str(&emit_str(&k));
            out.push_str(&emit_after_key(&v, indent));
            out.push('\n');
        }
        self.push_edit(file, at, at, out);
        Ok(())
    }

    fn append_to_empty(&mut self, file: &str, inserts: Vec<(String, Out)>) {
        let text = &self.a.doc(file).text;
        let mut out = String::new();
        if !text.is_empty() && !text.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&document(&Out::Map(inserts)));
        self.push_edit(file, text.len(), text.len(), out);
    }

    fn fill_empty_doc(&mut self, file: &str, pos: Pos, new: &Value) -> Result<()> {
        match shape(pos, new) {
            Out::Map(entries) => {
                if !entries.is_empty() {
                    self.append_to_empty(file, entries);
                }
                Ok(())
            }
            other => {
                if self.a.doc(file).has_comments() {
                    return Err(export_only(format!(
                        "{file} has comments and would be rewritten"
                    )));
                }
                self.whole.insert(file.to_string(), document(&other));
                Ok(())
            }
        }
    }

    fn replace(
        &mut self,
        file: &str,
        node: &Node,
        slot: Slot<'_>,
        pos: Pos,
        new: &Value,
        allow_loss: bool,
    ) -> Result<()> {
        let out = shape(pos, new);
        let doc = self.a.doc(file);
        match slot {
            Slot::Root => {
                if doc.has_comments() && !allow_loss {
                    return Err(export_only(format!(
                        "{file} has comments and would be rewritten"
                    )));
                }
                self.whole.insert(file.to_string(), document(&out));
            }
            Slot::Entry(e) => {
                let (start, end) = (e.key_end, node.end.max(e.key_end));
                if !doc.text[start..node.start.max(start)]
                    .trim_start()
                    .starts_with(':')
                {
                    return Err(export_only(format!(
                        "{file}: unusual layout for `{}`",
                        yaml::safe_key(&e.key)
                    )));
                }
                if !allow_loss && doc.has_comment_in(start, end) {
                    return Err(export_only(format!(
                        "{file}: `{}` contains comments that would be lost",
                        yaml::safe_key(&e.key)
                    )));
                }
                let indent = self.key_indent(file, e.key_start)?;
                let text = match emit_flow_seq(&out) {
                    Some(flow) if node.flow => format!(": {flow}"),
                    _ => emit_after_key(&out, indent),
                };
                self.push_edit(file, start, end, text);
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Vec<Op>> {
        let mut ops: Vec<Op> = self.dirs.into_iter().map(Op::MkDir).collect();
        for (file, edits) in self.edits {
            if self.whole.contains_key(&file) {
                return Err(export_only(format!("{file}: conflicting edits")));
            }
            ops.push(Op::Write(
                file.clone(),
                apply_edits(&self.a.doc(&file).text, edits, &file)?,
            ));
        }
        for (file, content) in self.whole {
            ops.push(Op::Write(file, content));
        }
        for (file, entries) in self.new_files {
            ops.push(Op::Write(file, document(&Out::Map(entries))));
        }
        ops.extend(self.deletes.into_iter().map(Op::Delete));
        Ok(ops)
    }
}

fn apply_edits(text: &str, edits: Vec<TextEdit>, file: &str) -> Result<String> {
    // Merge insertions at one point in generation order (inner before outer).
    let mut merged: Vec<TextEdit> = Vec::new();
    for e in edits {
        if e.start == e.end {
            if let Some(m) = merged
                .iter_mut()
                .find(|m| m.start == e.start && m.end == e.end)
            {
                m.text.push_str(&e.text);
                continue;
            }
        }
        merged.push(e);
    }
    merged.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));
    let mut out = text.to_string();
    let mut limit = usize::MAX;
    for e in merged {
        if e.end > limit {
            return Err(export_only(format!("{file}: overlapping edits")));
        }
        out.replace_range(e.start..e.end, &e.text);
        limit = e.start;
    }
    Ok(out)
}
