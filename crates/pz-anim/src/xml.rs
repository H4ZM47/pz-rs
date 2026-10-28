//! A small DOM for the AnimSet XML plus `zombie.util.PZXmlUtil.parseXml`:
//! `x_include` (splices another file's elements in front) and `x_extends`
//! (merges this file over a parent file, matching child elements by tag
//! and `x_name`, or by position among same-tag siblings).

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
    /// Comments and processing instructions: they only matter as
    /// non-text siblings.
    Other,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug)]
pub struct XmlError(pub String);

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XmlError {}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// `getAttribute` (missing attributes read as "").
    pub fn attr_or_empty(&self, name: &str) -> &str {
        self.attr(name).unwrap_or("")
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// `getTextContent`.
    pub fn text_content(&self) -> String {
        let mut s = String::new();
        fn walk(e: &Element, s: &mut String) {
            for c in &e.children {
                match c {
                    Node::Text(t) => s.push_str(t),
                    Node::Element(e) => walk(e, s),
                    Node::Other => {}
                }
            }
        }
        walk(self, &mut s);
        s
    }

    /// `Node.normalize()`: merges adjacent text and drops empty text.
    pub fn normalize(&mut self) {
        let mut out: Vec<Node> = Vec::with_capacity(self.children.len());
        for c in self.children.drain(..) {
            match c {
                Node::Text(t) => {
                    if t.is_empty() {
                        continue;
                    }
                    if let Some(Node::Text(prev)) = out.last_mut() {
                        prev.push_str(&t);
                    } else {
                        out.push(Node::Text(t));
                    }
                }
                Node::Element(mut e) => {
                    e.normalize();
                    out.push(Node::Element(e));
                }
                o => out.push(o),
            }
        }
        self.children = out;
    }

    fn is_text_only(&self) -> bool {
        let mut result = false;
        for c in &self.children {
            let is_text = matches!(c, Node::Text(t) if !t.chars().all(char::is_whitespace));
            if !is_text {
                return false;
            }
            result = true;
        }
        result
    }
}

// ---- parser ----

pub fn parse(src: &str) -> Result<Element, XmlError> {
    let src = src.replace("\r\n", "\n").replace('\r', "\n");
    let mut p = Parser { s: src.as_bytes(), i: 0 };
    p.skip_prolog()?;
    let mut root = p.element()?;
    root.normalize();
    Ok(root)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err<T>(&self, m: &str) -> Result<T, XmlError> {
        Err(XmlError(format!("{m} at byte {}", self.i)))
    }

    fn starts(&self, pat: &str) -> bool {
        self.s[self.i..].starts_with(pat.as_bytes())
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn skip_past(&mut self, pat: &str) -> Result<(), XmlError> {
        match find(&self.s[self.i..], pat.as_bytes()) {
            Some(k) => {
                self.i += k + pat.len();
                Ok(())
            }
            None => self.err(&format!("unterminated, expected {pat}")),
        }
    }

    fn skip_prolog(&mut self) -> Result<(), XmlError> {
        if self.starts("\u{feff}") {
            self.i += 3;
        }
        loop {
            self.skip_ws();
            if self.starts("<?") {
                self.skip_past("?>")?;
            } else if self.starts("<!--") {
                self.skip_past("-->")?;
            } else if self.starts("<!DOCTYPE") {
                self.skip_past(">")?;
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String, XmlError> {
        let st = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_whitespace() || c == b'>' || c == b'/' || c == b'=' {
                break;
            }
            self.i += 1;
        }
        if st == self.i {
            return self.err("expected a name");
        }
        Ok(String::from_utf8_lossy(&self.s[st..self.i]).into_owned())
    }

    fn element(&mut self) -> Result<Element, XmlError> {
        if !self.starts("<") {
            return self.err("expected <");
        }
        self.i += 1;
        let tag = self.name()?;
        let mut e = Element { tag, ..Default::default() };
        loop {
            self.skip_ws();
            if self.starts("/>") {
                self.i += 2;
                return Ok(e);
            }
            if self.starts(">") {
                self.i += 1;
                break;
            }
            let k = self.name()?;
            self.skip_ws();
            if !self.starts("=") {
                return self.err("expected =");
            }
            self.i += 1;
            self.skip_ws();
            let q = self.s.get(self.i).copied().unwrap_or(0);
            if q != b'"' && q != b'\'' {
                return self.err("expected quote");
            }
            self.i += 1;
            let st = self.i;
            while self.i < self.s.len() && self.s[self.i] != q {
                self.i += 1;
            }
            let raw = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
            self.i += 1;
            let v = decode_entities(&raw).replace(['\t', '\n'], " ");
            e.attrs.push((k, v));
        }
        loop {
            if self.i >= self.s.len() {
                return self.err("unterminated element");
            }
            if self.starts("</") {
                self.skip_past(">")?;
                return Ok(e);
            }
            if self.starts("<!--") {
                self.skip_past("-->")?;
                e.children.push(Node::Other);
            } else if self.starts("<![CDATA[") {
                self.i += 9;
                let st = self.i;
                self.skip_past("]]>")?;
                e.children.push(Node::Text(String::from_utf8_lossy(&self.s[st..self.i - 3]).into_owned()));
            } else if self.starts("<?") {
                self.skip_past("?>")?;
                e.children.push(Node::Other);
            } else if self.starts("<") {
                let c = self.element()?;
                e.children.push(Node::Element(c));
            } else {
                let st = self.i;
                while self.i < self.s.len() && self.s[self.i] != b'<' {
                    self.i += 1;
                }
                let raw = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
                e.children.push(Node::Text(decode_entities(&raw)));
            }
        }
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(a) = rest.find('&') {
        out.push_str(&rest[..a]);
        rest = &rest[a..];
        let Some(semi) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let ent = &rest[1..semi];
        let rep = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---- PZXmlUtil ----

/// `PZXmlUtil.parseXml(file)`: the file with its `x_include` and
/// `x_extends` resolved.
pub fn parse_xml_file(path: &Path) -> Result<Element, XmlError> {
    let real = resolve_case_insensitive(path).unwrap_or_else(|| path.to_path_buf());
    let text = std::fs::read_to_string(&real).map_err(|e| XmlError(format!("{}: {e}", real.display())))?;
    let mut root = parse(&text).map_err(|e| XmlError(format!("{}: {e}", real.display())))?;
    root = include_another_file(root, &real)?;
    let ext = root.attr_or_empty("x_extends").to_string();
    if ext.trim().is_empty() {
        return Ok(root);
    }
    let ext_path = real.parent().unwrap_or(Path::new("")).join(&ext);
    let extended = parse_xml_file(&ext_path)?;
    Ok(resolve(&root, &extended))
}

fn include_another_file(root: Element, file: &Path) -> Result<Element, XmlError> {
    let inc = root.attr_or_empty("x_include").to_string();
    if inc.trim().is_empty() {
        return Ok(root);
    }
    let inc_path = file.parent().unwrap_or(Path::new("")).join(&inc);
    let inc_root = parse_xml_file(&inc_path)?;
    if inc_root.tag != root.tag {
        return Ok(root);
    }
    let mut copy = root;
    let spliced: Vec<Node> = inc_root.children.into_iter().filter(|n| matches!(n, Node::Element(_))).collect();
    copy.children.splice(0..0, spliced);
    copy.normalize();
    Ok(copy)
}

/// `PZXmlUtil.resolve(root, parent)`.
pub fn resolve(root: &Element, parent: &Element) -> Element {
    if root.is_text_only() {
        return root.clone();
    }
    let mut attrs: Vec<(String, String)> = parent.attrs.clone();
    for (k, v) in &root.attrs {
        match attrs.iter_mut().find(|(pk, _)| pk == k) {
            Some(slot) => slot.1 = v.clone(),
            None => attrs.push((k.clone(), v.clone())),
        }
    }
    let mut parent_table = TagTable::create(parent);
    let child_table = TagTable::create(root);
    parent_table.resolve_with(child_table);
    Element {
        tag: root.tag.clone(),
        attrs,
        children: parent_table.resolved.into_iter().map(|e| Node::Element(e.element)).collect(),
    }
}

struct NamedTagEntry {
    tag: String,
    name: String,
    element: Element,
    index: usize,
}

impl NamedTagEntry {
    fn unique_key(&self) -> String {
        if self.name.chars().all(char::is_whitespace) {
            format!("node_{}", self.index)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Default)]
struct TagTable {
    /// tag → (unique key → position in `resolved`)
    tags: HashMap<String, HashMap<String, usize>>,
    resolved: Vec<NamedTagEntry>,
}

impl TagTable {
    fn create(root: &Element) -> TagTable {
        let mut t = TagTable::default();
        for e in root.elements() {
            let index = t.tags.get(&e.tag).map(|m| m.len()).unwrap_or(0);
            let entry = NamedTagEntry { tag: e.tag.clone(), name: e.attr_or_empty("x_name").to_string(), element: e.clone(), index };
            t.add_entry(entry);
        }
        t
    }

    fn add_entry(&mut self, entry: NamedTagEntry) {
        let key = entry.unique_key();
        let tag = entry.tag.clone();
        self.resolved.push(entry);
        let pos = self.resolved.len() - 1;
        self.tags.entry(tag).or_default().insert(key, pos);
    }

    fn get_entry(&self, child: &NamedTagEntry) -> Option<usize> {
        let named = self.tags.get(&child.tag)?;
        if child.name.chars().all(char::is_whitespace) {
            // The original searches the map values for the same index; the
            // indices are unique per tag, so the hash order does not matter.
            return named.values().copied().find(|&p| self.resolved[p].index == child.index);
        }
        named.get(&child.name).copied()
    }

    fn resolve_with(&mut self, child: TagTable) {
        for c in child.resolved {
            match self.get_entry(&c) {
                None => self.add_entry(c),
                Some(p) => {
                    let merged = resolve(&c.element, &self.resolved[p].element);
                    self.resolved[p].element = merged;
                }
            }
        }
    }
}

/// Opens `path` even when its case differs from the file on disk (the
/// game data was authored on a case-insensitive file system).
pub fn resolve_case_insensitive(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let mut cur = PathBuf::new();
    for comp in path.components() {
        let next = cur.join(comp.as_os_str());
        if next.exists() {
            cur = next;
            continue;
        }
        let want = comp.as_os_str().to_string_lossy().to_lowercase();
        let dir = if cur.as_os_str().is_empty() { PathBuf::from(".") } else { cur.clone() };
        let found = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().to_lowercase() == want)?;
        cur = cur.join(found.file_name());
    }
    Some(cur)
}
