//! `zombie.scripting.ScriptParser`: the brace/comma block format used by
//! `mods/default.txt` (and by the script files `pz-scripts` will load).
//!
//! All indices are byte offsets; the delimiters are ASCII, so slicing at them
//! matches Java's UTF-16 indices.

use crate::java;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Value {
    pub string: String,
}

impl Value {
    /// `getKey`: text before the first '=', or the whole string.
    pub fn key(&self) -> &str {
        match self.string.find('=') {
            Some(p) => &self.string[..p],
            None => &self.string,
        }
    }

    /// `getValue`: text after the first '=', or "".
    pub fn value(&self) -> &str {
        match self.string.find('=') {
            Some(p) => &self.string[p + 1..],
            None => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Block(usize),
    Value(usize),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub kind: String,
    pub id: Option<String>,
    /// Values and child blocks in source order (indices into the two lists).
    pub elements: Vec<Element>,
    pub values: Vec<Value>,
    pub children: Vec<Block>,
    pub comment: Option<String>,
}

impl Block {
    /// `getBlock(type, id)`.
    pub fn block(&self, kind: &str, id: Option<&str>) -> Option<&Block> {
        self.children.iter().find(|b| b.kind == kind && b.id.as_deref() == id)
    }

    /// `getValue(key)`: first value with an '=' after position 0 whose
    /// trimmed key equals `key`.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.values.iter().find(|v| v.string.find('=').is_some_and(|p| p > 0) && java::trim(v.key()) == key)
    }

    pub fn add_block(&mut self, kind: &str, id: Option<&str>) -> &mut Block {
        let block = Block { kind: kind.to_string(), id: id.map(str::to_string), ..Block::default() };
        self.elements.push(Element::Block(self.children.len()));
        self.children.push(block);
        self.children.last_mut().unwrap()
    }

    pub fn add_value(&mut self, key: &str, value: &str) {
        self.elements.push(Element::Value(self.values.len()));
        self.values.push(Value { string: format!("{key} = {value}") });
    }

    /// `setValue(key, value)`.
    pub fn set_value(&mut self, key: &str, value: &str) {
        let found = self.values.iter().position(|v| v.string.find('=').is_some_and(|p| p > 0) && java::trim(v.key()) == key);
        match found {
            Some(i) => self.values[i].string = format!("{key} = {value}"),
            None => self.add_value(key, value),
        }
    }

    /// `prettyPrint(indent, sb, eol, indentation)`.
    pub fn pretty_print(&self, indent: usize, sb: &mut String, eol: &str, indentation: &str) {
        sb.push_str(&indentation.repeat(indent));
        if let Some(c) = self.comment.as_deref().filter(|c| !java::is_null_or_whitespace(c)) {
            sb.push_str(c);
            sb.push_str(eol);
            sb.push_str(&indentation.repeat(indent));
        }
        sb.push_str(&self.kind);
        if let Some(id) = &self.id {
            sb.push(' ');
            sb.push_str(id);
        }
        sb.push_str(eol);
        sb.push_str(&indentation.repeat(indent));
        sb.push('{');
        sb.push_str(eol);
        self.pretty_print_elements(indent + 1, sb, eol, indentation);
        sb.push_str(&indentation.repeat(indent));
        sb.push('}');
        sb.push_str(eol);
    }

    /// `prettyPrintElements(indent, sb, eol, indentation)`.
    pub fn pretty_print_elements(&self, indent: usize, sb: &mut String, eol: &str, indentation: &str) {
        let mut prev: Option<&Element> = None;
        for e in &self.elements {
            match e {
                Element::Block(i) => {
                    if prev.is_some() {
                        sb.push_str(eol);
                    }
                    self.children[*i].pretty_print(indent, sb, eol, indentation);
                }
                Element::Value(i) => {
                    if matches!(prev, Some(Element::Block(_))) {
                        sb.push_str(eol);
                    }
                    sb.push_str(&indentation.repeat(indent));
                    sb.push_str(java::trim(&self.values[*i].string));
                    sb.push(',');
                    sb.push_str(eol);
                }
            }
            prev = Some(e);
        }
    }
}

/// `ScriptParser.readBlock(s, start, block)`; returns the index after the
/// block's closing brace (or `s.len()`).
pub fn read_block(s: &str, mut start: usize, block: &mut Block) -> usize {
    let bytes = s.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                let header = java::trim(&s[start..i]);
                let ss = java::split_ws(header);
                let mut child = Block {
                    kind: ss.first().cloned().unwrap_or_default(),
                    id: ss.get(1).cloned(),
                    ..Block::default()
                };
                let end = read_block(s, i + 1, &mut child);
                block.elements.push(Element::Block(block.children.len()));
                block.children.push(child);
                // Java's `continue` still runs the loop's `++i`, so the char
                // right after the closing brace is never examined.
                i = end;
                start = end;
            }
            b'}' => return i + 1,
            b',' => {
                block.elements.push(Element::Value(block.values.len()));
                block.values.push(Value { string: s[start..i].to_string() });
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// `ScriptParser.parse(s)`.
pub fn parse(s: &str) -> Block {
    let mut block = Block::default();
    read_block(s, 0, &mut block);
    block
}

/// `StringBuilder.lastIndexOf(str, from)`: last match starting at or before
/// `from`.
fn last_index_of(s: &str, pat: &str, from: isize) -> isize {
    if from < 0 {
        return -1;
    }
    let max_start = (from as usize).min(s.len().saturating_sub(pat.len()));
    if s.len() < pat.len() {
        return -1;
    }
    let b = s.as_bytes();
    let p = pat.as_bytes();
    let mut i = max_start as isize;
    while i >= 0 {
        let u = i as usize;
        if &b[u..u + p.len()] == p {
            return i;
        }
        i -= 1;
    }
    -1
}

/// `ScriptParser.stripComments(text)`: removes `/* ... */` comments,
/// handling nesting by scanning backwards from the last `*/`.
pub fn strip_comments(text: &str) -> String {
    let mut sb = text.to_string();
    let mut end = last_index_of(&sb, "*/", sb.len() as isize);
    while end != -1 {
        let mut start = last_index_of(&sb, "/*", end - 1);
        if start == -1 {
            break;
        }
        let mut inner_end = last_index_of(&sb, "*/", end - 1);
        while inner_end > start {
            let inner_start = start;
            start = last_index_of(&sb, "/*", start - 2);
            if start == -1 {
                break;
            }
            inner_end = last_index_of(&sb, "*/", inner_start - 2);
        }
        if start == -1 {
            break;
        }
        sb.replace_range(start as usize..(end + 2) as usize, "");
        end = last_index_of(&sb, "*/", start);
    }
    sb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mod_list() {
        let text = "VERSION = 1,\n\nmods\n{\n    mod = A,\n    mod = B\\,\n}\nmaps\n{\n}\n";
        let joined: String = java::read_lines(text).concat();
        let b = parse(&strip_comments(&joined));
        assert_eq!(b.value("VERSION").map(|v| java::trim(v.value())), Some("1"));
        let mods = b.block("mods", None).unwrap();
        let ids: Vec<_> = mods.values.iter().map(|v| java::trim(v.value()).to_string()).collect();
        assert_eq!(ids, ["A", "B\\"]);
        assert!(b.block("maps", None).unwrap().values.is_empty());
    }

    #[test]
    fn strips_nested_comments() {
        assert_eq!(strip_comments("a/* x /* y */ z */b"), "ab");
        assert_eq!(strip_comments("a/* x */b/* y */c"), "abc");
        assert_eq!(strip_comments("a */ b"), "a */ b");
    }

    #[test]
    fn pretty_print_round_trip() {
        let mut b = Block::default();
        b.set_value("VERSION", "1");
        let m = b.add_block("mods", None);
        m.add_value("mod", "A");
        b.add_block("maps", None);
        let mut s = String::new();
        b.pretty_print_elements(0, &mut s, "\n", "    ");
        assert_eq!(s, "VERSION = 1,\n\nmods\n{\n    mod = A,\n}\n\nmaps\n{\n}\n");
    }
}
