//! A deliberately small YAML layer: parsing into a span-carrying tree,
//! conversion to JSON values, emission, and comment detection.
//!
//! Supported: block and flow mappings/sequences, plain, quoted and block
//! scalars, comments. Rejected: anchors, aliases, tags, complex keys,
//! duplicate keys, multiple documents and excessive nesting. Plain scalars
//! resolve with the YAML 1.2 core schema (so `yes`/`on` are strings).

use anyhow::{bail, Result};
use saphyr_parser::{Event, Parser, ScalarStyle, Span};
use serde_json::{Map, Number, Value};

/// Maximum nesting depth of collections inside one file.
pub const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Quoted,
    Block,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Scalar { value: String, style: Style },
    Seq(Vec<Node>),
    Map(Vec<Entry>),
}

/// A parsed node. `start..end` is the byte range of the node's own text
/// (for block scalars, trailing blank lines are excluded).
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub flow: bool,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub key_start: usize,
    pub key_end: usize,
    pub key_line: usize,
    pub value: Node,
}

/// A parsed file.
#[derive(Debug, Clone)]
pub struct Doc {
    pub text: String,
    /// `None` for a document with no content (empty or only comments).
    pub root: Option<Node>,
    /// Byte offsets of every comment's `#`.
    pub comments: Vec<usize>,
}

impl Node {
    pub fn is_null(&self) -> bool {
        matches!(&self.kind, Kind::Scalar { value, style: Style::Plain } if is_null_plain(value))
    }
}

impl Doc {
    pub fn has_comment_in(&self, start: usize, end: usize) -> bool {
        let i = self.comments.partition_point(|&c| c < start);
        self.comments.get(i).is_some_and(|&c| c < end)
    }

    pub fn has_comments(&self) -> bool {
        !self.comments.is_empty()
    }
}

fn is_null_plain(s: &str) -> bool {
    matches!(s, "" | "~" | "null" | "Null" | "NULL")
}

struct Builder<'a> {
    text: &'a str,
    parser: Parser<'a, saphyr_parser::StrInput<'a>>,
    /// Byte offset of every char index, plus one past the end.
    offsets: Vec<usize>,
    scalars: Vec<(usize, usize)>,
}

impl<'a> Builder<'a> {
    fn byte(&self, char_index: usize) -> usize {
        self.offsets[char_index.min(self.offsets.len() - 1)]
    }

    fn next(&mut self) -> Result<(Event<'a>, Span)> {
        match self.parser.next_event() {
            Some(Ok(ev)) => Ok(ev),
            Some(Err(e)) => bail!(
                "line {} column {}: {}",
                e.marker().line(),
                e.marker().col() + 1,
                e.info()
            ),
            None => bail!("unexpected end of YAML"),
        }
    }

    fn node(&mut self, ev: Event<'a>, span: Span, depth: usize) -> Result<Node> {
        let line = span.start.line();
        let start = self.byte(span.start.index());
        if depth > MAX_DEPTH {
            bail!("line {line}: nesting deeper than {MAX_DEPTH} levels");
        }
        match ev {
            Event::Alias(_) => bail!("line {line}: YAML aliases are not supported"),
            Event::Scalar(value, style, anchor, tag) => {
                reject_anchor_tag(line, anchor, tag.is_some())?;
                let raw_end = self.byte(span.end.index());
                let (style, mut end) = match style {
                    ScalarStyle::Plain => (Style::Plain, raw_end),
                    // The parser's span for quoted scalars runs past trailing
                    // comments; find the closing quote ourselves.
                    ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => {
                        (Style::Quoted, quoted_end(self.text, start).min(raw_end))
                    }
                    ScalarStyle::Literal | ScalarStyle::Folded => (Style::Block, raw_end),
                };
                if style == Style::Block {
                    while end > start && self.text.as_bytes()[end - 1].is_ascii_whitespace() {
                        end -= 1;
                    }
                }
                self.scalars
                    .push((start, if style == Style::Block { raw_end } else { end }));
                Ok(Node {
                    kind: Kind::Scalar {
                        value: value.into_owned(),
                        style,
                    },
                    start,
                    end,
                    flow: false,
                })
            }
            Event::SequenceStart(anchor, tag) => {
                reject_anchor_tag(line, anchor, tag.is_some())?;
                let flow = span.end.index() > span.start.index();
                let mut items = Vec::new();
                let end;
                loop {
                    let (ev, sp) = self.next()?;
                    if matches!(ev, Event::SequenceEnd) {
                        end = if flow {
                            self.byte(sp.start.index() + 1)
                        } else {
                            items.last().map_or(start, |n: &Node| n.end)
                        };
                        break;
                    }
                    items.push(self.node(ev, sp, depth + 1)?);
                }
                Ok(Node {
                    kind: Kind::Seq(items),
                    start,
                    end,
                    flow,
                })
            }
            Event::MappingStart(anchor, tag) => {
                reject_anchor_tag(line, anchor, tag.is_some())?;
                let flow = span.end.index() > span.start.index();
                let mut entries: Vec<Entry> = Vec::new();
                let end;
                loop {
                    let (ev, sp) = self.next()?;
                    if matches!(ev, Event::MappingEnd) {
                        end = if flow {
                            self.byte(sp.start.index() + 1)
                        } else {
                            entries.last().map_or(start, |e| e.value.end.max(e.key_end))
                        };
                        break;
                    }
                    let key_line = sp.start.line();
                    let key_node = self.node(ev, sp, depth + 1)?;
                    let Kind::Scalar { value: key, .. } = key_node.kind else {
                        bail!("line {key_line}: only scalar mapping keys are supported");
                    };
                    if key == "<<" {
                        bail!("line {key_line}: YAML merge keys are not supported");
                    }
                    if let Some(prev) = entries.iter().find(|e| e.key == key) {
                        bail!(
                            "duplicate key `{}` on lines {} and {key_line}",
                            safe_key(&key),
                            prev.key_line
                        );
                    }
                    let (ev, sp) = self.next()?;
                    let mut value = self.node(ev, sp, depth + 1)?;
                    if value.start == value.end && value.is_null() {
                        // An omitted value is reported at the key; place it after the colon.
                        let colon = self.text[key_node.end..]
                            .find(':')
                            .map_or(key_node.end, |i| key_node.end + i + 1);
                        value.start = colon;
                        value.end = colon;
                    }
                    entries.push(Entry {
                        key,
                        key_start: key_node.start,
                        key_end: key_node.end,
                        key_line,
                        value,
                    });
                }
                Ok(Node {
                    kind: Kind::Map(entries),
                    start,
                    end,
                    flow,
                })
            }
            _ => bail!("line {line}: unexpected YAML structure"),
        }
    }
}

/// Byte offset just past the closing quote of the quoted scalar at `start`.
fn quoted_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let q = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if q == b'"' => i += 2,
            b if b == q => {
                if q == b'\'' && bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                } else {
                    return i + 1;
                }
            }
            _ => i += 1,
        }
    }
    text.len()
}

fn reject_anchor_tag(line: usize, anchor: usize, tagged: bool) -> Result<()> {
    if anchor != 0 {
        bail!("line {line}: YAML anchors are not supported");
    }
    if tagged {
        bail!("line {line}: YAML tags are not supported; write secret references as {{env: NAME}} or {{file: name}}");
    }
    Ok(())
}

/// Key text that is safe to show in errors: IDs and field names pass, other
/// keys are elided because they might be misplaced secret material.
pub fn safe_key(key: &str) -> &str {
    if !key.is_empty()
        && key.len() <= 80
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        key
    } else {
        "<key>"
    }
}

/// Parses one YAML document.
pub fn parse(text: &str) -> Result<Doc> {
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    let mut b = Builder {
        text,
        parser: Parser::new_from_str(text),
        offsets,
        scalars: Vec::new(),
    };
    let mut root = None;
    let mut documents = 0;
    loop {
        let (ev, sp) = b.next()?;
        match ev {
            Event::StreamStart | Event::DocumentEnd | Event::Nothing => {}
            Event::StreamEnd => break,
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    bail!(
                        "line {}: multiple YAML documents are not supported",
                        sp.start.line()
                    );
                }
                let (ev, sp) = b.next()?;
                let node = b.node(ev, sp, 0)?;
                root = (!(node.is_null() && node.start == node.end)).then_some(node);
            }
            _ => bail!("unexpected YAML structure"),
        }
    }
    b.scalars.sort_unstable();
    let comments = find_comments(text, &b.scalars);
    Ok(Doc {
        text: text.to_string(),
        root,
        comments,
    })
}

fn find_comments(text: &str, scalars: &[(usize, usize)]) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r')) {
            let k = scalars.partition_point(|&(s, _)| s <= i);
            let inside = k > 0 && scalars[k - 1].1 > i;
            if !inside {
                out.push(i);
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Converts a node to JSON using the YAML 1.2 core schema.
pub fn to_json(node: &Node) -> Value {
    match &node.kind {
        Kind::Scalar { value, style } => scalar_json(value, *style),
        Kind::Seq(items) => Value::Array(items.iter().map(to_json).collect()),
        Kind::Map(entries) => Value::Object(
            entries
                .iter()
                .map(|e| (e.key.clone(), to_json(&e.value)))
                .collect::<Map<_, _>>(),
        ),
    }
}

fn scalar_json(value: &str, style: Style) -> Value {
    if style != Style::Plain {
        return Value::String(value.to_string());
    }
    if is_null_plain(value) {
        return Value::Null;
    }
    match value {
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    let digits = value.strip_prefix(['-', '+']).unwrap_or(value);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = value.trim_start_matches('+').parse::<u64>() {
            return Value::Number(n.into());
        }
        if let Ok(n) = value.parse::<i64>() {
            return Value::Number(n.into());
        }
    }
    if is_float(value) {
        if let Some(n) = value.parse::<f64>().ok().and_then(Number::from_f64) {
            return Value::Number(n);
        }
    }
    Value::String(value.to_string())
}

fn is_float(s: &str) -> bool {
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (int, frac) = match mantissa.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (mantissa, None),
    };
    let all_digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok = all_digits(int)
        && frac.is_none_or(all_digits)
        && (!int.is_empty() || frac.is_some_and(|f| !f.is_empty()));
    let exp_ok = exp.is_none_or(|e| {
        let e = e.strip_prefix(['-', '+']).unwrap_or(e);
        !e.is_empty() && all_digits(e)
    });
    mantissa_ok && exp_ok && (frac.is_some() || exp.is_some())
}

// ---------------------------------------------------------------------------
// Emission

/// Emits a string as a plain scalar when that is unambiguous, otherwise as a
/// JSON-escaped double-quoted scalar (valid YAML).
pub fn emit_str(s: &str) -> String {
    let reserved = matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n" | "~"
    );
    let first_ok = s
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'/' || b == b'_');
    // Digit-led text stays plain only when no YAML version reads it as a
    // number, timestamp or sexagesimal value (e.g. IP addresses).
    let digit_ok = !s.starts_with(|c: char| c.is_ascii_digit())
        || (s.matches('.').count() > 1 && s.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
    let body_ok = s.bytes().all(|b| {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b' ' | b'_' | b'.' | b'/' | b'-' | b'@' | b'+' | b'(' | b')' | b':'
            )
    });
    let colon_ok = !s.contains(": ") && !s.ends_with(':');
    let resolves_to_self = scalar_json(s, Style::Plain) == Value::String(s.to_string());
    if first_ok
        && digit_ok
        && body_ok
        && colon_ok
        && !reserved
        && resolves_to_self
        && !s.ends_with(' ')
        && !s.contains("  ")
    {
        s.to_string()
    } else {
        serde_json::to_string(s).expect("string serialization")
    }
}

pub fn emit_scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => emit_str(s),
        Value::Array(a) if a.is_empty() => "[]".into(),
        Value::Object(o) if o.is_empty() => "{}".into(),
        other => serde_json::to_string(other).expect("json"),
    }
}

pub fn indent(n: usize) -> String {
    " ".repeat(n)
}

/// An ordered value ready for emission (see `schema::shape`).
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Scalar(Value),
    Seq(Vec<Out>),
    Map(Vec<(String, Out)>),
}

impl Out {
    /// `{env: NAME}` / `{file: name}` secret references stay on one line.
    fn secret_ref(&self) -> Option<String> {
        match self {
            Out::Map(m) if m.len() == 1 && matches!(m[0].0.as_str(), "env" | "file") => {
                match &m[0].1 {
                    Out::Scalar(v @ Value::String(_)) => {
                        Some(format!("{{{}: {}}}", m[0].0, emit_scalar(v)))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn is_inline(&self) -> bool {
        self.secret_ref().is_some()
            || match self {
                Out::Scalar(_) => true,
                Out::Seq(a) => a.is_empty(),
                Out::Map(m) => m.is_empty(),
            }
    }

    fn inline(&self) -> String {
        if let Some(s) = self.secret_ref() {
            return s;
        }
        match self {
            Out::Scalar(v) => emit_scalar(v),
            Out::Seq(_) => "[]".into(),
            Out::Map(_) => "{}".into(),
        }
    }
}

/// Text that follows a key: `: value` or `:\n` plus an indented block
/// (without a final newline).
pub fn emit_after_key(v: &Out, key_indent: usize) -> String {
    if v.is_inline() {
        format!(": {}", v.inline())
    } else {
        let mut out = String::from(":\n");
        emit_block(v, key_indent + 2, &mut out);
        out.truncate(out.trim_end_matches('\n').len());
        out
    }
}

/// Emits a value as block lines at `ind`, each ending in `\n`.
pub fn emit_block(v: &Out, ind: usize, out: &mut String) {
    match v {
        Out::Map(entries) if !entries.is_empty() => {
            for (k, child) in entries {
                out.push_str(&indent(ind));
                out.push_str(&emit_str(k));
                out.push_str(&emit_after_key(child, ind));
                out.push('\n');
            }
        }
        Out::Seq(items) if !items.is_empty() => {
            for item in items {
                out.push_str(&indent(ind));
                out.push_str("- ");
                if item.is_inline() {
                    out.push_str(&item.inline());
                    out.push('\n');
                } else {
                    let mut inner = String::new();
                    emit_block(item, ind + 2, &mut inner);
                    out.push_str(&inner[ind + 2..]);
                }
            }
        }
        other => {
            out.push_str(&indent(ind));
            out.push_str(&other.inline());
            out.push('\n');
        }
    }
}

/// Flow form for a sequence of scalars, e.g. `[a/b, c/d]`.
pub fn emit_flow_seq(v: &Out) -> Option<String> {
    match v {
        Out::Seq(items) if items.iter().all(|i| matches!(i, Out::Scalar(_))) => Some(format!(
            "[{}]",
            items.iter().map(Out::inline).collect::<Vec<_>>().join(", ")
        )),
        _ => None,
    }
}

/// Byte offset of the start of the line containing `pos`.
pub fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// Byte offset just past the newline ending the line containing `pos`
/// (or the end of text).
pub fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_follow_core_schema() {
        let doc = parse("a: yes\nb: 010\nc: 1.5\nd: '1'\ne:\nf: -3\ng: 1e3\nh: .5\n").unwrap();
        let v = to_json(doc.root.as_ref().unwrap());
        assert_eq!(v["a"], "yes");
        assert_eq!(v["b"], 10);
        assert_eq!(v["c"], 1.5);
        assert_eq!(v["d"], "1");
        assert_eq!(v["e"], Value::Null);
        assert_eq!(v["f"], -3);
        assert_eq!(v["g"], 1000.0);
        assert_eq!(v["h"], 0.5);
    }

    #[test]
    fn comments_exclude_scalar_content() {
        let text = "a: \"x # y\" # real\nb: c#d\n# line\nd: |\n  # literal\n";
        let doc = parse(text).unwrap();
        let found: Vec<&str> = doc.comments.iter().map(|&i| &text[i..i + 6]).collect();
        assert_eq!(found, vec!["# real", "# line"]);
    }

    #[test]
    fn node_ranges_are_exact() {
        let text = "a: \"x # y\"  # c\nb: 'it''s' # y\nc: [a, \"b\" ] # z\nd: {k: v}  # m\ne: |  # k\n  t\n\nf: plain # p\ng:\n  - x # q\n";
        let doc = parse(text).unwrap();
        let Kind::Map(entries) = &doc.root.as_ref().unwrap().kind else {
            panic!()
        };
        let got: Vec<&str> = entries
            .iter()
            .map(|e| &text[e.value.start..e.value.end])
            .collect();
        assert_eq!(
            got,
            vec![
                "\"x # y\"",
                "'it''s'",
                "[a, \"b\" ]",
                "{k: v}",
                "t",
                "plain",
                "- x"
            ]
        );
        assert_eq!(doc.comments.len(), 7);
    }

    #[test]
    fn emitted_strings_reparse_identically() {
        for s in [
            "plain",
            "yes",
            "No",
            "123",
            "1.5",
            "a: b",
            "a #b",
            " lead",
            "trail ",
            "",
            "https://x.example/a?b=c",
            "quote\"s",
            "new\nline",
            "-dash",
            "~",
            "@at",
            "é",
            "null",
            "a:",
            "tab\there",
            "192.168.1.10",
            "1.2",
            "1:20",
            "0x1F",
            "2026-01-01",
            "1.2.3",
            "{x}",
            "[y]",
            "*alias",
            "&anchor",
            "!tag",
            "%dir",
        ] {
            let text = format!("k: {}\n", emit_str(s));
            let doc = parse(&text).unwrap_or_else(|e| panic!("{s:?}: {e}"));
            assert_eq!(to_json(doc.root.as_ref().unwrap())["k"], s, "{text}");
        }
    }

    #[test]
    fn rejects_unsupported_constructs() {
        for bad in [
            "a: &x 1\nb: *x\n",
            "a: !!str 1\n",
            "a: 1\na: 2\n",
            "? [a]\n: 1\n",
            "a: 1\n---\nb: 2\n",
            "<<: {a: 1}\n",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let deep = "[".repeat(MAX_DEPTH + 2) + &"]".repeat(MAX_DEPTH + 2);
        assert!(parse(&deep).is_err());
    }
}
